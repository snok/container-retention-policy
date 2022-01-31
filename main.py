from __future__ import annotations

import asyncio
from asyncio import Semaphore
from datetime import datetime
from enum import Enum
from fnmatch import fnmatch
from sys import argv
from typing import TYPE_CHECKING, NamedTuple, Optional
from urllib.parse import quote_from_bytes

from dateparser import parse
from httpx import AsyncClient
from pydantic import BaseModel, conint, validator

if TYPE_CHECKING:
    from typing import Any

    from httpx import Response

BASE_URL = 'https://api.github.com'


class ImageName(NamedTuple):
    """
    We need to store both the raw image names and url-encoded image names.

    The raw images names are used for logging, while the url-encoded
    images names are sent in our payloads to the Github API.
    """

    value: str
    encoded: str


class TimestampType(str, Enum):
    """
    The timestamp-to-use defines how to filter down images for deletion.
    """

    UPDATED_AT = 'updated_at'
    CREATED_AT = 'created_at'


class AccountType(str, Enum):
    """
    The user's account type defines which endpoints to use.
    """

    ORG = 'org'
    PERSONAL = 'personal'


deleted: list[str] = []
failed: list[str] = []
needs_github_assistance: list[str] = []
GITHUB_ASSISTANCE_MSG = (
    'Publicly visible package versions with more than '
    '5000 downloads cannot be deleted. '
    'Contact GitHub support for further assistance.'
)


async def list_org_package_versions(
    *, org_name: str, image_name: ImageName, http_client: AsyncClient
) -> list[dict[str, Any]]:
    """
    List image versions, for an organization.

    :param org_name: The name of the organization.
    :param image_name: The name of the container image.
    :param http_client: HTTP client.
    :return: List of image objects.
    """
    response = await http_client.get(
        f'{BASE_URL}/orgs/{org_name}/packages/container/{image_name.encoded}/versions?per_page=100'
    )
    response.raise_for_status()
    return response.json()


async def list_package_versions(*, image_name: ImageName, http_client: AsyncClient) -> list[dict]:
    """
    List image versions, for a personal account.

    :param image_name: The name of the container image.
    :param http_client: HTTP client.
    :return: List of image objects.
    """
    response = await http_client.get(f'{BASE_URL}/user/packages/container/{image_name.encoded}/versions?per_page=100')
    response.raise_for_status()
    return response.json()


def post_deletion_output(*, response: Response, image_name: ImageName, version_id: int) -> None:
    """
    Output a little info to the user.
    """
    image_name_with_tag = f'{image_name.value}:{version_id}'
    if response.is_error:
        if response.status_code == 400 and response.json()['message'] == GITHUB_ASSISTANCE_MSG:
            # Output the names of these images in one block at the end
            needs_github_assistance.append(image_name_with_tag)
        else:
            failed.append(image_name_with_tag)
            print(
                f'\nCouldn\'t delete {image_name_with_tag}.\n'
                f'Status code: {response.status_code}\nResponse: {response.json()}\n'
            )
    else:
        deleted.append(image_name_with_tag)
        print(f'Deleted old image: {image_name_with_tag}')


async def delete_org_package_versions(
    *, org_name: str, image_name: ImageName, version_id: int, http_client: AsyncClient, semaphore: Semaphore
) -> None:
    """
    Delete an image version, for an organization.

    :param org_name: The name of the org.
    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/orgs/{org_name}/packages/container/{image_name.encoded}/versions/{version_id}'
    await semaphore.acquire()
    try:
        response = await http_client.delete(url)
    finally:
        semaphore.release()
    post_deletion_output(response=response, image_name=image_name, version_id=version_id)


async def delete_package_versions(
    *, image_name: ImageName, version_id: int, http_client: AsyncClient, semaphore: Semaphore
) -> None:
    """
    Delete an image version, for a personal account.

    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/user/packages/container/{image_name.encoded}/versions/{version_id}'
    await semaphore.acquire()
    try:
        response = await http_client.delete(url)
    finally:
        semaphore.release()
    post_deletion_output(response=response, image_name=image_name, version_id=version_id)


class GithubAPI:
    """
    Provide a unified API, regardless of account type.
    """

    @staticmethod
    async def list_package_versions(
        *, account_type: AccountType, org_name: Optional[str], image_name: ImageName, http_client: AsyncClient
    ) -> list[dict[str, Any]]:
        if account_type != AccountType.ORG:
            return await list_package_versions(image_name=image_name, http_client=http_client)
        assert isinstance(org_name, str)
        return await list_org_package_versions(org_name=org_name, image_name=image_name, http_client=http_client)

    @staticmethod
    async def delete_package(
        *,
        account_type: AccountType,
        org_name: Optional[str],
        image_name: ImageName,
        version_id: int,
        http_client: AsyncClient,
        semaphore: Semaphore,
    ) -> None:
        if account_type != AccountType.ORG:
            return await delete_package_versions(
                image_name=image_name, version_id=version_id, http_client=http_client, semaphore=semaphore
            )
        assert isinstance(org_name, str)
        return await delete_org_package_versions(
            org_name=org_name,
            image_name=image_name,
            version_id=version_id,
            http_client=http_client,
            semaphore=semaphore,
        )


class Inputs(BaseModel):
    image_names: list[ImageName]
    cut_off: datetime
    timestamp_to_use: TimestampType
    account_type: AccountType
    org_name: Optional[str]
    untagged_only: bool
    skip_tags: list[str]
    keep_at_least: conint(ge=0) = 0  # type: ignore[valid-type]
    filter_tags: list[str]
    filter_include_untagged: bool = True

    @validator('image_names', pre=True)
    def parse_image_names(cls, v: str) -> list[ImageName]:
        """
        Return an ImageName for each images name received.

        The image_name can be one or multiple image names, and should be comma-separated.

        For images with special characters in the name (e.g., `/`), we *must* url-encode
        the image names before passing them to the Github API, so we save both the url-
        encoded and raw value to a named tuple.
        """
        return [
            ImageName(img_name.strip(), quote_from_bytes(img_name.strip().encode('utf-8'), safe=''))
            for img_name in v.split(',')
        ]

    @validator('skip_tags', 'filter_tags', pre=True)
    def parse_comma_separate_string_as_list(cls, v: str) -> list[str]:
        if not v:
            return []
        else:
            return [i.strip() for i in v.split(',')]

    @validator('cut_off', pre=True)
    def parse_human_readable_datetime(cls, v: str) -> datetime:
        parsed_cutoff = parse(v)
        if not parsed_cutoff:
            raise ValueError(f"Unable to parse '{v}'")
        elif parsed_cutoff.tzinfo is None or parsed_cutoff.tzinfo.utcoffset(parsed_cutoff) is None:
            raise ValueError('Timezone is required for the cut-off')
        return parsed_cutoff

    @validator('org_name', pre=True)
    def validate_org_name(cls, v: str, values: dict) -> Optional[str]:
        if values['account_type'] == AccountType.ORG and not v:
            raise ValueError('org-name is required when account-type is org')
        if v:
            return v
        return None


async def get_and_delete_old_versions(image_name: ImageName, inputs: Inputs, http_client: AsyncClient) -> None:
    """
    Delete old package versions for an image name.

    This function contains more or less all our logic.
    """
    versions = await GithubAPI.list_package_versions(
        account_type=inputs.account_type, org_name=inputs.org_name, image_name=image_name, http_client=http_client
    )

    # Trim the version list to the n'th element we want to keep
    if inputs.keep_at_least > 0:
        versions = versions[inputs.keep_at_least :]

    # Define list of deletion-tasks to append to
    tasks = []

    # Iterate through dicts of image versions
    sem = Semaphore(50)

    async with sem:
        for version in versions:

            # Parse either the update-at timestamp, or the created-at timestamp
            # depending on which on the user has specified that we should use
            updated_or_created_at = parse(version[inputs.timestamp_to_use.value])

            if not updated_or_created_at:
                print(f'Skipping image version {version["id"]}. Unable to parse timestamps.')
                continue

            if inputs.cut_off < updated_or_created_at:
                # Skipping because it's above our datetime cut-off
                # we're only looking to delete containers older than some timestamp
                continue

            # Load the tags for the individual image we're processing
            if (
                'metadata' in version
                and 'container' in version['metadata']
                and 'tags' in version['metadata']['container']
            ):
                image_tags = version['metadata']['container']['tags']
            else:
                image_tags = []

            if inputs.untagged_only and image_tags:
                # Skipping because no tagged images should be deleted
                # We could proceed if image_tags was empty, but it's not
                continue

            if not image_tags and not inputs.filter_include_untagged:
                # Skipping, because the filter_include_untagged setting is False
                continue

            delete_image = not inputs.filter_tags
            for filter_tag in inputs.filter_tags:
                # One thing to note here is that we use fnmatch to support wildcards.
                # A filter-tags setting of 'some-tag-*' should match to both
                # 'some-tag-1' and 'some-tag-2'.
                if any(fnmatch(tag, filter_tag) for tag in image_tags):
                    delete_image = True
                    break

            for skip_tag in inputs.skip_tags:
                if any(fnmatch(tag, skip_tag) for tag in image_tags):
                    # Skipping because this image version is tagged with a protected tag
                    delete_image = False

            if delete_image:
                tasks.append(
                    asyncio.create_task(
                        GithubAPI.delete_package(
                            account_type=inputs.account_type,
                            org_name=inputs.org_name,
                            image_name=image_name,
                            version_id=version['id'],
                            http_client=http_client,
                            semaphore=sem,
                        )
                    )
                )

    if not tasks:
        print(f'No more versions to delete for {image_name.value}')

    await asyncio.gather(*tasks)


async def main(
    account_type: str,
    org_name: str,
    image_names: str,
    timestamp_to_use: str,
    cut_off: str,
    token: str,
    untagged_only: str,
    skip_tags: str,
    keep_at_least: str,
    filter_tags: str,
    filter_include_untagged: str,
) -> None:
    """
    Delete old image versions.

    See action.yml for additional descriptions of each parameter.

    The argument order matters. They are fed to the script from the action, in order.

    All arguments are either strings or empty strings. We properly
    parse types and values in the Inputs pydantic model.

    :param account_type: Account type. must be 'org' or 'personal'.
    :param org_name: The name of the org. Required if account type is 'org'.
    :param image_names: The image names to delete versions for. Can be a single
                        image name, or multiple comma-separated image names.
    :param timestamp_to_use: Which timestamp to base our cut-off on. Can be 'updated_at' or 'created_at'.
    :param cut_off: Can be a human readable relative time like '2 days ago UTC', or a timestamp.
                            Must contain a reference to the timezone.
    :param token: The personal access token to authenticate with.
    :param untagged_only: Whether to only delete untagged images.
    :param skip_tags: Comma-separated list of tags to not delete.
        Supports wildcard '*', '?', '[seq]' and '[!seq]' via Unix shell-style wildcards
    :param keep_at_least: Number of images to always keep
    :param filter_tags: Comma-separated list of tags to consider for deletion.
        Supports wildcard '*', '?', '[seq]' and '[!seq]' via Unix shell-style wildcards
    :param filter_include_untagged: Whether to consider untagged images for deletion.
    """
    inputs = Inputs(
        image_names=image_names,
        account_type=account_type,
        org_name=org_name,
        timestamp_to_use=timestamp_to_use,
        cut_off=cut_off,
        untagged_only=untagged_only,
        skip_tags=skip_tags,
        keep_at_least=keep_at_least,
        filter_tags=filter_tags,
        filter_include_untagged=filter_include_untagged,
    )
    async with AsyncClient(
        headers={'accept': 'application/vnd.github.v3+json', 'Authorization': f'Bearer {token}'}
    ) as client:
        tasks = [
            asyncio.create_task(get_and_delete_old_versions(image_name, inputs, client))
            for image_name in inputs.image_names
        ]
        await asyncio.gather(*tasks)

    if needs_github_assistance:
        # Print a human readable list of public images we couldn't handle
        image_list = '\n\t- ' + '\n\t- '.join(needs_github_assistance)
        msg = (
            'The follow images are public and have more than 5000 downloads. '
            f'These cannot be deleted via the Github API:\n{image_list}\n\n'
            f'If you still want to delete these images, contact Github support.\n\n'
            'See https://docs.github.com/en/rest/reference/packages for more info.'
        )
        print(msg)

    # Then add it to the action outputs
    print('\nSetting action outputs:')
    for name, l in [
        ('needs-github-assistance', needs_github_assistance),
        ('deleted', deleted),
        ('failed', failed),
    ]:
        comma_separated_list = ','.join(l)
        print(f'::set-output name={name}::{comma_separated_list}')


if __name__ == '__main__':
    asyncio.run(main(*argv[1:]))
