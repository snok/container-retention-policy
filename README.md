[![release](https://img.shields.io/github/v/release/snok/container-retention-policy)](https://github.com/snok/container-retention-policy/releases/latest)
[![coverage](https://codecov.io/gh/snok/drf-openapi-tester/branch/master/graph/badge.svg)](https://codecov.io/gh/snok/container-retention-policy)

# 📘 GHCR Container Retention Policy

A GitHub Action for deleting old image versions from the GitHub container registry.

Storage isn't free and registries can often get bloated with unused images. Having a retention policy to prevent clutter
makes sense in most cases.

Supports both organizational and personal accounts.

# Content

- [Usage](#usage)
- [Examples](#examples)
- [Parameters](#parameters)
- [Nice to knows](#nice-to-knows)
- [Contributing](#contributing)

# Usage

To use the action, simply add it to your GitHub workflow, like this:

```yaml
- uses: snok/container-retention-policy@v2
  with:
    image-names: dev, web, test*
    cut-off: two hours ago UTC+2
    timestamp-to-use: updated_at
    account-type: org
    org-name: google
    keep-at-least: 1
    skip-tags: latest
    token: ${{ secrets.PAT }}
```

Notice image-names supports wildcards.

You could run this as
a [scheduled event](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#schedule), or as a part
of an existing workflow, but for the sake of inspiration, it might also make sense for you to trigger it with a:

- [workflow_dispatch](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#workflow_dispatch):
  trigger it manually in the GitHub repo UI when needed
- [workflow_run](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#workflow_run): have it run
  as clean-up after another key workflow completes
- or triggering it with a
  webhook ([repository_dispatch](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#repository_dispatch))

# Examples

For an organization, a full example might look something like this:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: "0 0 * * *"  # every day at midnight


jobs:
  clean-ghcr:
    name: Delete old unused container images
    runs-on: ubuntu-latest
    steps:
      - name: Delete 'dev' containers older than a week
        uses: snok/container-retention-policy@v2
        with:
          image-names: python-dev, js-dev
          cut-off: A week ago UTC
          account-type: org
          org-name: my-org
          keep-at-least: 1
          untagged-only: true
          token: ${{ secrets.PAT }}

      - name: Delete all test containers older than a month, using a wildcard
        uses: snok/container-retention-policy@v2
        with:
          image-names: python-test*, js-test*
          cut-off: One month ago UTC
          account-type: org
          org-name: my-org
          keep-at-least: 1
          skip-tags: latest
          token: ${{ secrets.PAT }}
```

While for a personal account, something like this might do:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: '0 0 0 * *'  # the first day of the month

jobs:
  clean-ghcr:
    name: Delete old unused container images
    runs-on: ubuntu-latest
    steps:
      - name: Delete old images
        uses: snok/container-retention-policy@v2
        with:
          image-names: dev/*
          cut-off: One month ago UTC
          keep-at-least: 1
          account-type: personal
          token: ${{ secrets.PAT }}
```

An example showing 2 different retention policies based on image tags format:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: '0 0 0 * *'  # the first day of the month

jobs:
  clean-ghcr:
    name: Delete old unused container images
    runs-on: ubuntu-latest
    steps:
      - name: Delete old released images
        uses: snok/container-retention-policy@v2
        with:
          image-names: dev/*
          cut-off: One month ago UTC
          keep-at-least: 5
          filter-tags: "v*.*.*"
          account-type: personal
          token: ${{ secrets.PAT }}
      - name: Delete old pre-release images
        uses: snok/container-retention-policy@v2
        with:
          image-names: dev/*
          cut-off: One week ago UTC
          keep-at-least: 1
          filter-tags: "rc*", "dev*"
          account-type: personal
          token: ${{ secrets.PAT }}
```

# Parameters

## image-names

* **Required**: `Yes`
* **Example**: `image-names: image1,image2,image3` or just `image*`

The names of the container images you want to delete old versions for. Takes one or several container image names as a
comma separated list, and supports wildcards. The action will fetch all packages available, and filter
down the list of packages to handle based on the image name input.

## cut-off

* **Required**: `Yes`
* **Example**: `cut-off: 1 week ago UTC`

The timezone-aware datetime you want to delete container versions that are older than.

We use [dateparser](https://dateparser.readthedocs.io/en/latest/) to parse the cut-off specified. This means you should
be able to specify your cut-off in relative human readable terms like `Two hours ago UTC`, or by using a normal
timestamp.

The parsed datetime **must** contain a timezone.

## timestamp-to-use

* **Required**: `Yes`
* **Example**: `timestamp-to-use: created_at`
* **Default**: `updated_at`
* **Valid choices**: `updated_at` or `created_at`

Which timestamp to use when comparing the cut-off to the container version.

Must be `created_at` or `updated_at`. The timestamp to use determines how we filter container versions.

## account-type

* **Required**: `Yes`
* **Example**: `account-type: personal`
* **Valid choices**: `org` or `personal`

The account type of the account running the action. The account type determines which API endpoints to use in the GitHub
API.

## org-name

* **Required**: `Only if account type is org`
* **Example**: `org-name: google`

The name of your organization.

## token

* **Required**: `Yes`
* **Example**: `token: ${{ secrets.PAT }}`

For the token, you need to pass
a [personal access token](https://docs.github.com/en/github/authenticating-to-github/keeping-your-account-and-data-secure/creating-a-personal-access-token)
with access to the container registry. Specifically, you need to grant it the following scopes:

- `read:packages`, and
- `delete:packages`

## keep-at-least

* **Required**: `No`
* **Default**: `0`
* **Example**: `keep-at-least: 5`

How many versions to keep no matter what. Defaults to 0, meaning all versions older than the `cut-off` date may be deleted.

Setting this to a larger value ensures that the specified number of recent versions are always retained, regardless of their age. Useful for images that are not updated very often.

If used together with `filter-tags` parameter, `keep-at-least` number of image tags will be skipped from the resulting filtered set, which makes it possible to apply different retention policies based on image tag format.

## untagged-only

* **Required**: `No`
* **Default**: `false`

Restricts image deletion to images without any tags, if enabled.

## skip-tags

* **Required**: `No`
* **Example**: `latest, v*`

Restrict deletions to images without specific tags, if specified.

Supports Unix-shell style wildcards, i.e 'v*' to match all tags starting with 'v'.

## filter-tags

* **Required**: `No`
* **Example**: `sha-*`

Comma-separated list of tags to consider for deletion.

Supports Unix-shell style wildcards, i.e 'sha-*' to match all tags starting with 'sha-'.

## filter-include-untagged

* **Required**: `No`
* **Default**: `true`

Whether to consider untagged images for deletion.

## dry-run

* **Required**: `No`
* **Default**: `false`

Prints output showing imaages which would be deleted but does not actually delete any images.

# Outputs

## deleted

Comma-separated list of `image-name:version-id` for each image deleted.

## failed

Comma-separated list of images that we weren't able to delete. Check
logs for responses.

## needs-github-assistance

When a container image version is public and reaches
5,000 downloads, the image version can no longer
be deleted via the Github API.

If you run into this issue, you can access the names and versions
of the relevant images by calling `${{ steps.<the-id-of-the-deletion-step>.outputs.needs-github-assistance }}`.

The names and versions are output as a comma-separate list,
like `"name1:tag1,name2:tag2"`.

# Nice to knows

* The GitHub API restricts us to fetching 100 image versions per image name, so if your registry isn't 100% clean after
  the first job, don't be alarmed.

* If you accidentally delete something you shouldn't have, GitHub apparently has a 30 day grace period before actually
  deleting your image version.
  See [these docs](https://docs.github.com/en/rest/reference/packages#restore-package-version-for-an-organization)
  for the information you need to restore your data.

# Contributing

Please do 👏
