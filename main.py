from __future__ import annotations

import asyncio
import os
import re
from asyncio import Semaphore, Task
from datetime import datetime, timedelta
from enum import Enum
from fnmatch import fnmatch
from sys import argv
from typing import TYPE_CHECKING, Literal
from urllib.parse import quote_from_bytes

from dateparser import parse
from httpx import AsyncClient, TimeoutException
from pydantic import BaseModel, ValidationInfo, conint, field_validator

if TYPE_CHECKING:
    from httpx import Response

BASE_URL = 'https://api.github.com'


def encode_image_name(name: str) -> str:
    return quote_from_bytes(name.strip().encode('utf-8'), safe='')


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


class GithubTokenType(str, Enum):
    """The type of token to use to authenticate to GitHub."""

    GITHUB_TOKEN = 'github-token'
    # Personal Access Token (PAT)
    PAT = 'pat'


deleted: list[str] = []
failed: list[str] = []
needs_github_assistance: list[str] = []

GITHUB_ASSISTANCE_MSG = (
    'Publicly visible package versions with more than '
    '5000 downloads cannot be deleted. '
    'Contact GitHub support for further assistance.'
)


class PackageResponse(BaseModel):
    id: int
    name: str
    created_at: datetime
    updated_at: datetime | None


# This could be made into a setting if needed
MAX_SLEEP = 60 * 10  # 10 minutes


async def wait_for_rate_limit(*, response: Response, eligible_for_secondary_limit: bool = False) -> None:
    """
    Sleeps or terminates the workflow if we've hit rate limits.

    See docs on rate limits: https://docs.github.com/en/rest/rate-limit?apiVersion=2022-11-28.
    """
    if int(response.headers.get('x-ratelimit-remaining', 1)) == 0:
        ratelimit_reset = datetime.fromtimestamp(int(response.headers['x-ratelimit-reset']))
        delta = ratelimit_reset - datetime.now()

        if delta > timedelta(seconds=MAX_SLEEP):
            print(
                f'Rate limited for {delta} seconds. '
                f'Terminating workflow, since that\'s above the maximum allowed sleep time. '
                f'Retry the job manually, when the rate limit is refreshed.'
            )
            exit(1)
        elif delta > timedelta(seconds=0):
            print(f'Rate limit exceeded. Sleeping for {delta} seconds')
            await asyncio.sleep(delta.total_seconds())

    elif eligible_for_secondary_limit:
        # https://docs.github.com/en/rest/overview/resources-in-the-rest-api?apiVersion=2022-11-28#secondary-rate-limits
        # https://docs.github.com/en/rest/guides/best-practices-for-integrators#dealing-with-secondary-rate-limits
        if int(response.headers.get('retry-after', 1)) == 0:
            ratelimit_reset = datetime.fromtimestamp(int(response.headers['retry-after']))
            delta = ratelimit_reset - datetime.now()
            if delta > timedelta(seconds=MAX_SLEEP):
                print(
                    f'Rate limited for {delta} seconds. '
                    f'Terminating workflow, since that\'s above the maximum allowed sleep time. '
                    f'Retry the job manually, when the rate limit is refreshed.'
                )
                exit(1)
            elif delta > timedelta(seconds=0):
                print(f'Secondary Rate limit exceeded. Sleeping for {delta} seconds')
                await asyncio.sleep(delta.total_seconds())
        else:
            await asyncio.sleep(1)


async def get_all_pages(*, url: str, http_client: AsyncClient) -> list[dict]:
    """
    Accumulate all pages of a paginated API endpoint.

    :param url: The full API URL
    :param http_client: HTTP client.
    :return: List of objects.
    """
    result = []
    rel_regex = re.compile(r'<([^<>]*)>; rel="(\w+)"')
    rels = {'next': url}

    while 'next' in rels:
        response = await http_client.get(rels['next'])
        response.raise_for_status()
        result.extend(response.json())

        await wait_for_rate_limit(response=response)

        if link := response.headers.get('link'):
            rels = {rel: url for url, rel in rel_regex.findall(link)}
        else:
            break

    return result


async def list_org_packages(*, org_name: str, http_client: AsyncClient) -> list[PackageResponse]:
    """List all packages for an organization."""
    packages = await get_all_pages(
        url=f'{BASE_URL}/orgs/{org_name}/packages?package_type=container&per_page=100',
        http_client=http_client,
    )
    return [PackageResponse(**i) for i in packages]


async def list_packages(*, http_client: AsyncClient) -> list[PackageResponse]:
    """List all packages for a user."""
    packages = await get_all_pages(
        url=f'{BASE_URL}/user/packages?package_type=container&per_page=100',
        http_client=http_client,
    )
    return [PackageResponse(**i) for i in packages]


async def list_org_package_versions(
    *, org_name: str, image_name: str, http_client: AsyncClient
) -> list[PackageVersionResponse]:
    """List image versions, for an organization."""
    packages = await get_all_pages(
        url=f'{BASE_URL}/orgs/{org_name}/packages/container/{encode_image_name(image_name)}/versions?per_page=100',
        http_client=http_client,
    )
    return [PackageVersionResponse(**i) for i in packages]


async def list_package_versions(*, image_name: str, http_client: AsyncClient) -> list[PackageVersionResponse]:
    """List image versions for a user."""
    packages = await get_all_pages(
        url=f'{BASE_URL}/user/packages/container/{encode_image_name(image_name)}/versions?per_page=100',
        http_client=http_client,
    )
    return [PackageVersionResponse(**i) for i in packages]


class ContainerModel(BaseModel):
    tags: list[str]


class MetadataModel(BaseModel):
    package_type: Literal['container']
    container: ContainerModel


class PackageVersionResponse(BaseModel):
    id: int
    name: str
    metadata: MetadataModel
    created_at: datetime | None
    updated_at: datetime | None


def post_deletion_output(*, response: Response, image_name: str, version_id: int) -> None:
    """
    Output a little info to the user.
    """
    image_name_with_tag = f'{image_name}:{version_id}'
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


async def delete_package_version(
    url: str, semaphore: Semaphore, http_client: AsyncClient, image_name: str, version_id: int
) -> None:
    async with semaphore:
        try:
            response = await http_client.delete(url)
            await wait_for_rate_limit(response=response, eligible_for_secondary_limit=True)
            post_deletion_output(response=response, image_name=image_name, version_id=version_id)
        except TimeoutException as e:
            print(f'Request to delete {image_name} timed out with error `{e}`')


async def delete_org_package_versions(
    *,
    org_name: str,
    image_name: str,
    version_id: int,
    http_client: AsyncClient,
    semaphore: Semaphore,
) -> None:
    """
    Delete an image version, for an organization.

    :param org_name: The name of the org.
    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/orgs/{org_name}/packages/container/{encode_image_name(image_name)}/versions/{version_id}'
    await delete_package_version(
        url=url,
        semaphore=semaphore,
        http_client=http_client,
        image_name=image_name,
        version_id=version_id,
    )


async def delete_package_versions(
    *, image_name: str, version_id: int, http_client: AsyncClient, semaphore: Semaphore
) -> None:
    """
    Delete an image version, for a personal account.

    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/user/packages/container/{encode_image_name(image_name)}/versions/{version_id}'
    await delete_package_version(
        url=url,
        semaphore=semaphore,
        http_client=http_client,
        image_name=image_name,
        version_id=version_id,
    )


class GithubAPI:
    """
    Provide a unified API, regardless of account type.
    """

    @staticmethod
    async def list_packages(
        *, account_type: AccountType, org_name: str | None, http_client: AsyncClient
    ) -> list[PackageResponse]:
        if account_type != AccountType.ORG:
            return await list_packages(http_client=http_client)
        assert isinstance(org_name, str)
        return await list_org_packages(org_name=org_name, http_client=http_client)

    @staticmethod
    async def list_package_versions(
        *,
        account_type: AccountType,
        org_name: str | None,
        image_name: str,
        http_client: AsyncClient,
    ) -> list[PackageVersionResponse]:
        if account_type != AccountType.ORG:
            return await list_package_versions(image_name=image_name, http_client=http_client)
        assert isinstance(org_name, str)
        return await list_org_package_versions(org_name=org_name, image_name=image_name, http_client=http_client)

    @staticmethod
    async def delete_package(
        *,
        account_type: AccountType,
        org_name: str | None,
        image_name: str,
        version_id: int,
        http_client: AsyncClient,
        semaphore: Semaphore,
    ) -> None:
        if account_type != AccountType.ORG:
            return await delete_package_versions(
                image_name=image_name,
                version_id=version_id,
                http_client=http_client,
                semaphore=semaphore,
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
    token_type: GithubTokenType = GithubTokenType.PAT
    image_names: list[str]
    cut_off: datetime
    timestamp_to_use: TimestampType
    account_type: AccountType
    org_name: str | None = None
    untagged_only: bool
    skip_tags: list[str]
    keep_at_least: conint(ge=0) = 0  # type: ignore[valid-type]
    filter_tags: list[str]
    filter_include_untagged: bool = True
    dry_run: bool = False

    @staticmethod
    def _parse_comma_separate_string_as_list(v: str) -> list[str]:
        return [i.strip() for i in v.split(',')] if v else []

    @field_validator('skip_tags', 'filter_tags', mode='before')
    def parse_comma_separate_string_as_list(cls, v: str) -> list[str]:
        return cls._parse_comma_separate_string_as_list(v)

    @field_validator('image_names', mode='before')
    def validate_image_names(cls, v: str, values: ValidationInfo) -> list[str]:
        images = cls._parse_comma_separate_string_as_list(v)
        if 'token_type' in values.data:
            token_type = values.data['token_type']
            if token_type == GithubTokenType.GITHUB_TOKEN and len(images) != 1:
                raise ValueError('A single image name is required if token_type is github-token')
            if token_type == GithubTokenType.GITHUB_TOKEN and '*' in images[0]:
                raise ValueError('Wildcards are not allowed if token_type is github-token')
        return images

    @field_validator('cut_off', mode='before')
    def parse_human_readable_datetime(cls, v: str) -> datetime:
        parsed_cutoff = parse(v)
        if not parsed_cutoff:
            raise ValueError(f"Unable to parse '{v}'")
        elif parsed_cutoff.tzinfo is None or parsed_cutoff.tzinfo.utcoffset(parsed_cutoff) is None:
            raise ValueError('Timezone is required for the cut-off')
        return parsed_cutoff

    @field_validator('org_name', mode='before')
    def validate_org_name(cls, v: str, values: ValidationInfo) -> str | None:
        if 'account_type' in values.data and values.data['account_type'] == AccountType.ORG and not v:
            raise ValueError('org-name is required when account-type is org')
        if v:
            return v
        return None


async def get_and_delete_old_versions(image_name: str, inputs: Inputs, http_client: AsyncClient) -> None:
    """
    Delete old package versions for an image name.

    This function contains more or less all our logic.
    """
    versions = await GithubAPI.list_package_versions(
        account_type=inputs.account_type,
        org_name=inputs.org_name,
        image_name=image_name,
        http_client=http_client,
    )

    # Define list of deletion-tasks to append to
    tasks: list[Task] = []
    simulated_tasks = 0

    # Iterate through dicts of image versions
    sem = Semaphore(50)

    async with sem:
        for idx, version in enumerate(versions):
            # Parse either the update-at timestamp, or the created-at timestamp
            # depending on which on the user has specified that we should use
            updated_or_created_at = getattr(version, inputs.timestamp_to_use.value)

            if not updated_or_created_at:
                print(f'Skipping image version {version.id}. Unable to parse timestamps.')
                continue

            if inputs.cut_off < updated_or_created_at:
                # Skipping because it's above our datetime cut-off
                # we're only looking to delete containers older than some timestamp
                continue

            # Load the tags for the individual image we're processing
            if (
                hasattr(version, 'metadata')
                and hasattr(version.metadata, 'container')
                and hasattr(version.metadata.container, 'tags')
            ):
                image_tags = version.metadata.container.tags
            else:
                image_tags = []

            if inputs.untagged_only and image_tags:
                # Skipping because no tagged images should be deleted
                # We could proceed if image_tags was empty, but it's not
                continue

            if not image_tags and not inputs.filter_include_untagged:
                # Skipping, because the filter_include_untagged setting is False
                continue

            # If we got here, most probably we will delete image.
            # For pseudo-branching we set delete_image to true and
            # handle cases with delete image by tag filtering in separate pseudo-branch
            delete_image = not inputs.filter_tags
            for filter_tag in inputs.filter_tags:
                # One thing to note here is that we use fnmatch to support wildcards.
                # A filter-tags setting of 'some-tag-*' should match to both
                # 'some-tag-1' and 'some-tag-2'.
                if any(fnmatch(tag, filter_tag) for tag in image_tags):
                    delete_image = True
                    break

            if inputs.keep_at_least > 0:
                if idx + 1 - (len(tasks) + simulated_tasks) > inputs.keep_at_least:
                    delete_image = True
                else:
                    delete_image = False

            # Here we will handle exclusion case
            for skip_tag in inputs.skip_tags:
                if any(fnmatch(tag, skip_tag) for tag in image_tags):
                    # Skipping because this image version is tagged with a protected tag
                    delete_image = False

            if delete_image is True and inputs.dry_run:
                delete_image = False
                simulated_tasks += 1
                print(f'Would delete image {image_name}:{version.id}.')

            if delete_image:
                tasks.append(
                    asyncio.create_task(
                        GithubAPI.delete_package(
                            account_type=inputs.account_type,
                            org_name=inputs.org_name,
                            image_name=image_name,
                            version_id=version.id,
                            http_client=http_client,
                            semaphore=sem,
                        )
                    )
                )

    if not tasks:
        print(f'No more versions to delete for {image_name}')

    results = await asyncio.gather(*tasks, return_exceptions=True)

    for item in results:
        if isinstance(item, Exception):
            try:
                raise item
            except Exception as e:
                # Unhandled errors *shouldn't* occur
                print(
                    f'Unhandled exception raised at runtime: `{e}`. '
                    f'Please report this at https://github.com/snok/container-retention-policy/issues/new'
                )


def filter_image_names(all_packages: list[PackageResponse], image_names: list[str]) -> set[str]:
    """
    Filter package names by action input package names.

    The action input can contain wildcards and other patterns supported by fnmatch.

    The idea is that given a list: ['ab', 'ac', 'bb', 'ba'], and image names (from the action inputs): ['aa', 'b*'],
    this function should return ['ba', 'bb'].

    :param all_packages: List of packages received from the Github API
    :param image_names: List of image names the client wishes to delete from
    :return: The intersection of the two lists, returned as `ImageName` instances
    """

    packages_to_delete_from = set()

    # Iterate over image names from the action inputs and fnmatch to packages
    # contained in the users/orgs list of packages.
    for image_name in image_names:
        for package in all_packages:
            if fnmatch(package.name, image_name):
                packages_to_delete_from.add(package.name.strip())

    return packages_to_delete_from


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
    dry_run: str = 'false',
    token_type: str = 'pat',
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
    :param cut_off: Can be a human-readable relative time like '2 days ago UTC', or a timestamp.
                            Must contain a reference to the timezone.
    :param token: The personal access token to authenticate with.
    :param untagged_only: Whether to only delete untagged images.
    :param skip_tags: Comma-separated list of tags to not delete.
        Supports wildcard '*', '?', '[seq]' and '[!seq]' via Unix shell-style wildcards
    :param keep_at_least: Number of images to always keep
    :param filter_tags: Comma-separated list of tags to consider for deletion.
        Supports wildcard '*', '?', '[seq]' and '[!seq]' via Unix shell-style wildcards
    :param filter_include_untagged: Whether to consider untagged images for deletion.
    :param dry_run: Do not actually delete packages but print output showing which packages would
        have been deleted.
    :param token_type: Token passed into 'token'. Must be 'pat' or 'github-token'. If
                       'github-token' is used, then 'image_names` must be a single image,
                       and the image matches the package name from the repository where
                       this action is invoked.
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
        dry_run=dry_run,
        token_type=token_type,
    )
    async with AsyncClient(
        headers={'accept': 'application/vnd.github.v3+json', 'Authorization': f'Bearer {token}'}
    ) as client:
        if inputs.token_type == GithubTokenType.GITHUB_TOKEN:
            packages_to_delete_from = set(inputs.image_names)
        else:
            # Get all packages from the user or orgs account
            all_packages = await GithubAPI.list_packages(
                account_type=inputs.account_type,
                org_name=inputs.org_name,
                http_client=client,
            )

            # Filter existing image names by action inputs
            packages_to_delete_from = filter_image_names(all_packages, inputs.image_names)

        # Create tasks to run concurrently
        tasks = [
            asyncio.create_task(get_and_delete_old_versions(image_name, inputs, client))
            for image_name in packages_to_delete_from
        ]

        # Execute tasks
        await asyncio.gather(*tasks)

    if needs_github_assistance:
        # Print a human-readable list of public images we couldn't handle
        print('\n')
        print('─' * 110)
        image_list = '\n\t- ' + '\n\t- '.join(needs_github_assistance)
        msg = (
            '\nThe follow images are public and have more than 5000 downloads. '
            f'These cannot be deleted via the Github API:\n{image_list}\n\n'
            f'If you still want to delete these images, contact Github support.\n\n'
            'See https://docs.github.com/en/rest/reference/packages for more info.\n'
        )
        print(msg)
        print('─' * 110)

    # Then add it to the action outputs
    for name, l in [
        ('needs-github-assistance', needs_github_assistance),
        ('deleted', deleted),
        ('failed', failed),
    ]:
        comma_separated_list = ','.join(l)

        with open(os.environ['GITHUB_OUTPUT'], 'a') as f:
            f.write(f'{name}={comma_separated_list}\n')


if __name__ == '__main__':
    asyncio.run(main(*argv[1:]))
