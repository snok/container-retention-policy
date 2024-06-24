# v3.0.0

💥 Beware, this release breaks the API of the action to a large degree. It might be wise to run the action with `dry-run: true` after upgrading.

After a period of incrementally adopting features, the action arguments have become unnecessarily confusing and the API has become bloated. This release consolidates and streamlines the API.

The new release also adds support for a lot of new features, and fixes most long-standing issues.

**New features**

- Support for multi-arch images. See the new section in the [README.md](https://github.com/snok/container-retention-policy/blob/main/README.md#safely-handling-multi-platform-multi-arch-packages) for details.
- Support for GitHub app tokens ([docs](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app))
- Support for GitHub temporal tokens (`secrets.GITHUB_TOKEN`) ([docs](https://docs.github.com/en/actions/security-guides/automatic-token-authentication#about-the-github_token-secret))
- Proper handling of primary and secondary rate limits ([docs](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28))
- The available syntax for `image-names` and `image-tags` previously allowed wildcards (using the `*` character). We now also allow the `?` character to express a single-character wildcard. For example, the pattern `ca?` will match `car` and  `cat`. See [wildmatch docs](https://github.com/becheran/wildmatch) for details.
- Significant effort has been spent on improving the logging, to give better insights into what exactly is happening
- Updated license from `BSD-3` to `MIT`.

**Breaking changes**

- Over half of the arguments have changed. See the [migration guide](#migration-guide) below for details.
- The [`needs-assistance` output](https://github.com/snok/container-retention-policy/tree/575226aa6cf28ee190c6611e8cc20d545264f443?tab=readme-ov-file#needs-github-assistance) was deleted, since it seem unlikely to ever be used.
- We will not maintain mutable major and minor version tags for the action going forward. In other words, there will be no `v3` target for the action, just `v3.0.0` and other exact versions. In my experience, a mutable major version tag is not much safer than using `@main`. More precise tag tracking is safer for most, and pairs well with [dependabot](https://docs.github.com/en/code-security/dependabot/working-with-dependabot/keeping-your-actions-up-to-date-with-dependabot) if you don't want to track new versions yourself.

**Performance improvements**

- The action has been rewritten from a [composite action](https://docs.github.com/en/actions/creating-actions/creating-a-composite-action) to a [container action](https://docs.github.com/en/actions/creating-actions/creating-a-docker-container-action), and the total size of the new image is < 10Mi.
- The action would previously take ~30 seconds to initialize and would need a Python runtime. The action now starts in less than a second, and runs as a standalone binary.
- The runtime of the action has been reduced, and assuming we need to fetch less than 100 package versions, the action completes in, at most, a few seconds.

## Migration guide

- The `account-type` and `org-name` inputs have been replaced with `account`, which should be set to the literal string "user" if you previously used `account-type: personal` and to the organization name otherwise:

    ```diff
    - account-type: personal
    + account: user
    ```

    or

    ```diff
    - account-type: organization
    - org-name: acme
    + account: acme
    ```

- The `filter-tags` key was renamed to `image-tags`

    ```diff
    - filter-tags: *-prod
    + image-tags: *-prod
    ```

- The `token-type` input has been removed. If you previously used `token-type: github-token`, then you should make the following change:

    ```diff
    - token-type: github-token
    + token: ${{ secrets.GITHUB_TOKEN }}
    ```

  In other words, we've consolidated `token-type` and `token` into a single arg.

- The `skip-tags` input has been removed. If you previously used `skip-tags: latest`, you should now specify a negative glob pattern in `image-tags`.

    ```diff
    - filter-tags: l*
    - skip-tags: latest
    + image-tags: l*, !latest
    ```

  In other words, we've consolidated the two arguments, by adding support for the `!` operator, which means "not".

- The `filter-include-untagged` and `untagged-only` inputs were removed.

  `filter-include-untagged` previously enabled you to opt-out of deleting untagged images, while `untagged-only` would allow you to opt-out of deleting tagged images. This was a bit confusing, even for me.

  To make things simpler, these have been collapsed into one argument, called `tag-selection` which accepts the string values `tagged`, `untagged`, or `both`.

    ```diff
    - filter-include-untagged: true
    - untagged-only: false
    + tag-selection: both
    ```

    or

    ```diff
    - filter-include-untagged: true
    - untagged-only: true
    + tag-selection: untagged
    ```

- The `cut-off` input no longer accepts human-readable datetimes. Instead, it accepts the inputs listed [here](https://github.com/tailhook/humantime). For example:

    ```diff
    - cut-off: two hours and 5 minutes ago UTC+2
    + cut-off: 2h 5m
    ```

    or

    ```diff
    - cut-off: One week ago UTC
    + cut-off: 1w
    ```

  There is no longer timezone support built-into this option. All durations are relative to the current time, UTC.
