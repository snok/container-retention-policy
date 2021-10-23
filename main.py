from __future__ import annotations

import asyncio
from collections import namedtuple
from dataclasses import dataclass
from distutils.util import strtobool
from enum import Enum
from functools import partial
from sys import argv
from typing import TYPE_CHECKING
from urllib.parse import quote_from_bytes

from dateparser import parse
from httpx import AsyncClient

if TYPE_CHECKING:
    from datetime import datetime
    from typing import Any, Callable, Coroutine, Optional, Union

    from httpx import Response

BASE_URL = 'https://api.github.com'

ImageName = namedtuple('ImageName', ['value', 'encoded'])


class TimestampType(Enum):
    UPDATED_AT = 'updated_at'
    CREATED_AT = 'created_at'


class AccountType(Enum):
    ORG = 'org'
    PERSONAL = 'personal'


async def list_org_package_versions(org_name: str, image_name: ImageName, http_client: AsyncClient) -> list[dict]:
    """
    List image versions for an organization.

    :param org_name: The name of the organization.
    :param image_name: The name of the container image.
    :param http_client: HTTP client.
    :return: List of image objects.
    """
    response = await http_client.get(f'{BASE_URL}/orgs/{org_name}/packages/container/{image_name.encoded}/versions?per_page=100')
    response.raise_for_status()
    return response.json()


async def list_package_versions(image_name: ImageName, http_client: AsyncClient) -> list[dict]:
    """
    List image versions for a personal account.

    :param image_name: The name of the container image.
    :param http_client: HTTP client.
    :return: List of image objects.
    """
    response = await http_client.get(f'{BASE_URL}/user/packages/container/{image_name.encoded}/versions?per_page=100')
    response.raise_for_status()
    return response.json()


def post_deletion_output(response: Response, image_name: ImageName, version_id: int) -> None:
    """
    Output a little info to the user.
    """
    if response.is_error:
        print(f'\nCouldn\'t delete {image_name.value}:{version_id}.\nStatus code: {response.status_code}\nResponse: {response.json()}\n')
    else:
        print(f'Deleted old image: {image_name.value}:{version_id}')


async def delete_org_package_versions(org_name: str, image_name: ImageName, version_id: int, http_client: AsyncClient) -> None:
    """
    Delete an image version for an organization.

    :param org_name: The name of the org.
    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/orgs/{org_name}/packages/container/{image_name.encoded}/versions/{version_id}'
    response = await http_client.delete(url)
    post_deletion_output(response, image_name, version_id)


async def delete_package_versions(image_name: ImageName, version_id: int, http_client: AsyncClient) -> None:
    """
    Delete an image version for a personal account.

    :param image_name: The name of the container image.
    :param version_id: The ID of the image version we're deleting.
    :param http_client: HTTP client.
    :return: Nothing - the API returns a 204.
    """
    url = f'{BASE_URL}/user/packages/container/{image_name.encoded}/versions/{version_id}'
    response = await http_client.delete(url)
    post_deletion_output(response, image_name, version_id)


def get_image_version_tags(version: dict) -> list[str]:
    """
    Return the list of tags on a container image.
    """
    if 'metadata' in version and 'container' in version['metadata'] and 'tags' in version['metadata']['container']:
        return version['metadata']['container']['tags']
    return []


@dataclass
class Inputs:
    """
    Class holds validated inputs, and unifies the API for org- and personal functions.
    """

    parsed_cutoff: datetime
    timestamp_type: TimestampType
    account_type: AccountType
    untagged_only: bool
    skip_tags: list[str]
    keep_at_least: int
    org_name: Optional[str] = None

    def __post_init__(self) -> None:
        """
        Cast keep-at-least to int.
        """
        self.keep_at_least = int(self.keep_at_least)

    @property
    def is_org(self) -> bool:
        """
        Whether the account type is an org or not.
        """
        return self.account_type == AccountType.ORG

    @property
    def list_package_versions(self) -> Callable[[ImageName, Any], Coroutine[Any, Any, list[dict]]]:
        """
        Unify the API for package version list functions.
        """
        if self.is_org:
            return partial(list_org_package_versions, self.org_name)
        else:
            return list_package_versions

    @property
    def delete_package(self) -> Callable[[ImageName, int, Any], Coroutine[Any, Any, None]]:
        """
        Unify the API for package deletion functions.
        """
        if self.is_org:
            return partial(delete_org_package_versions, self.org_name)
        else:
            return delete_package_versions


async def get_and_delete_old_versions(image_name: ImageName, inputs: Inputs, http_client: AsyncClient) -> None:
    """
    Delete old package versions for an image name.
    """
    versions = await inputs.list_package_versions(image_name, http_client)

    if inputs.keep_at_least >= 0:
        versions = versions[inputs.keep_at_least :]

    tasks = []

    for version in versions:
        updated_or_created_at = parse(version[inputs.timestamp_type.value])

        if not updated_or_created_at:
            print(f'Skipping image version {version["id"]}. Unable to parse timestamps.')
            continue

        if updated_or_created_at > inputs.parsed_cutoff:
            # Skipping because it's not below the datetime cut-off
            continue

        image_tags = get_image_version_tags(version)

        if inputs.untagged_only and image_tags:
            # Skipping because no tagged images should be deleted
            continue

        if any(tag in inputs.skip_tags for tag in image_tags):
            # Skipping because this image version is tagged with a protected tag
            continue

        tasks.append(asyncio.create_task(inputs.delete_package(image_name, version['id'], http_client)))

    if not tasks:
        print(f'No more versions to delete for {image_name.value}')

    await asyncio.gather(*tasks)


def validate_inputs(
    account_type: str,
    org_name: str,
    timestamp_type: str,
    cut_off: str,
    untagged_only: Union[bool, str],
    skip_tags: Optional[str],
    keep_at_least: Optional[str],
) -> Inputs:
    """
    Perform basic validation on the incoming parameters and return an Inputs instance.
    """
    # For date parsing we use `dateparser`. If you're having issues getting this to work,
    # check out https://dateparser.readthedocs.io/en/latest/.
    if not (parsed_cutoff := parse(cut_off)):
        raise ValueError(f"Unable to parse '{cut_off}'")
    elif parsed_cutoff.tzinfo is None or parsed_cutoff.tzinfo.utcoffset(parsed_cutoff) is None:
        raise ValueError('Timezone is required for the cut-off')

    if account_type == 'org' and not org_name:
        raise ValueError('org-name is required when account-type is org')

    if isinstance(untagged_only, str):
        untagged_only_ = strtobool(untagged_only) == 1
    else:
        untagged_only_ = untagged_only

    if skip_tags is None:
        skip_tags_ = []
    else:
        skip_tags_ = [i.strip() for i in skip_tags.split(',')]

    if keep_at_least is None:
        keep_at_least_ = 0
    else:
        keep_at_least_ = int(keep_at_least)
        if keep_at_least_ < 0:
            raise ValueError('keep-at-least must be 0 or positive')

    return Inputs(
        parsed_cutoff=parsed_cutoff,
        timestamp_type=TimestampType(timestamp_type),
        account_type=AccountType(account_type),
        org_name=org_name if account_type == 'org' else None,
        untagged_only=untagged_only_,
        skip_tags=skip_tags_,
        keep_at_least=keep_at_least_,
    )


def parse_image_names(image_names: str) -> list[ImageName]:
    """
    Return an ImageName for each images name received.

    The image_name can be one or multiple image names, and should be comma-separated.
    For images with special characters in the name (e.g., `/`), we must url-encode
    the image names before passing them to the Github API, so we save both the url-
    encoded and raw value to a named tuple.
    """
    return [ImageName(img_name.strip(), quote_from_bytes(img_name.strip().encode('utf-8'), safe='')) for img_name in image_names.split(',')]


async def main(
    account_type: str,
    org_name: str,
    image_names: str,
    timestamp_type: str,
    cut_off: str,
    token: str,
    untagged_only: Union[bool, str] = False,
    skip_tags: Optional[str] = None,
    keep_at_least: Optional[str] = None,
) -> None:
    """
    Delete old image versions.

    See action.yml for additional descriptions of each parameter.

    :param account_type: Account type, must be 'org' or 'personal'.
    :param org_name: The name of the org. Required if account type is 'org'.
    :param image_names: The image names to delete versions for.
                        Can be a single image name, or multiple comma-separated image names.
    :param timestamp_type: Which timestamp to base our cut-off on. Can be 'updated_at' or 'created_at'.
    :param cut_off: Can be a human readable relative time like '2 days ago UTC', or a timestamp.
                            Must contain a reference to the timezone.
    :param token: The personal access token to authenticate with.
    :param untagged_only: Whether to only delete untagged images.
    :param skip_tags: Comma-separated list of tags to not delete.
    :param keep_at_least: Number of images to always keep
    """
    parsed_image_names: list[ImageName] = parse_image_names(image_names)
    inputs: Inputs = validate_inputs(account_type, org_name, timestamp_type, cut_off, untagged_only, skip_tags, keep_at_least)
    headers = {'accept': 'application/vnd.github.v3+json', 'Authorization': f'Bearer {token}'}

    async with AsyncClient(headers=headers) as http_client:
        await asyncio.gather(
            *(asyncio.create_task(get_and_delete_old_versions(image_name, inputs, http_client)) for image_name in parsed_image_names)
        )


if __name__ == '__main__':
    asyncio.run(main(*argv[1:]))
