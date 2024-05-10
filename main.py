
deleted: list[str] = []
failed: list[str] = []
needs_github_assistance: list[str] = []

GITHUB_ASSISTANCE_MSG = (
    'Publicly visible package versions with more than '
    '5000 downloads cannot be deleted. '
    'Contact GitHub support for further assistance.'
)


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
