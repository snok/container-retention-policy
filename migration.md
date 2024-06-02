- The `account-type` and `org-name` inputs have been replaced with `account`, which should be set to `user` if you previously used `account-type: personal` and to the organization name otherwise:

-   ```diff
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

- The `token-type` input has been removed. If you previously used `token-type: github-token`, set `'github-token'` as your `token` input value instead.

    ```diff
    - token-type: github-token
    + token: ${{ secrets.GITHUB_TOKEN }}
    ```

- The `skip-tags` input has been removed. If you previously used `skip-tags: latest`, you should now specify a negative glob pattern in `image-tags`.

    ```diff
    - filter-tags: l*
    - skip-tags: latest
    + image-tags: l*, !latest
    ```

- The `filter-include-untagged` and `untagged-only` inputs were removed. `filter-include-untagged` previously enabled you to opt-out of deleting untagged images, while `untagged-only` would allow you to opt-out of deleting tagged images. These have been replaced by `tag-selection` which accepts the string values `tagged`, `untagged`, or `both`.

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

- The available syntax for `image-names` and `image-tags` has previously allowed wildcards, using the `*` character. We now also allow the `?` character to express a single-character wildcard. For example, the pattern `ca?` will match `car` and  `cat`. See [wildmatch docs](https://github.com/becheran/wildmatch) for details.

- The `cut-off` input no longer accepts human-readable datetimes. Instead, it accepts the inputs listed [here](https://crates.io/crates/duration-str). For example:

    ```diff
    - cut-off: two hours and 5 minutes ago UTC+2
    + cut-off: 2h + 5m
    ```

    or

    ```diff
    - cut-off: One week ago UTC
    + cut-off: 1w
    ```

  There is no longer timezone support built-into this option. All durations are relative to the current time, UTC.

- The action now (indirectly) supports multi-arch/multi-platform packages. Take a look at the new README for details.

needs-assistance-output was deleted
