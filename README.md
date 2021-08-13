[![release](https://img.shields.io/github/release/sondrelg/container-retention-policy.svg)](https://github.com/sondrelg/container-retention-policy/releases/latest)
[![coverage](https://codecov.io/gh/snok/drf-openapi-tester/branch/master/graph/badge.svg)](https://codecov.io/gh/sondrelg/container-retention-policy)

# 📘 GHCR Container Retention Policy

A Github Action for deleting old image versions from the Github container registry.

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

To use the action, simply add it to your Github workflow, like this:

```yaml
- uses: sondrelg/container-retention-policy@v0.1
  with:
    image-names: dev, web, test
    cut-off: two hours ago UTC+2
    timestamp-to-use: updated_at
    account-type: org
    org-name: google
    token: ${{ secrets.PAT }}
```

You could run this as
a [scheduled event](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#schedule), or as a part
of an existing workflow, but for the sake of inspiration, it might also make sense for you to trigger it with a:

- [workflow_dispatch](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#workflow_dispatch): trigger it manually in the Github repo UI when needed
- [workflow_run](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#workflow_run): have it run as clean-up after another key workflow completes
- or triggering it with a
  webhook ([repository_dispatch](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#repository_dispatch))

# Examples

For an organization, a full example might look something like this:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: '0 0 * * *'  # every day at midnight


jobs:
  clean-ghcr:
    name: Delete old unused container images
    runs-on: ubuntu-latest
    steps:
      - name: Delete 'dev' containers older than a week
        uses: sondrelg/container-retention-policy@v0.1
        with:
          image-names: python-dev, js-dev
          cut-off: A week ago UTC
          account-type: org
          org-name: my-org
          token: ${{ secrets.PAT }}
  
      - name: Delete 'test' containers older than a month
        uses: sondrelg/container-retention-policy@v0.1
        with:
          image-names: python-test, js-test
          cut-off: One month ago UTC
          account-type: org
          org-name: my-org
          token: ${{ secrets.PAT }}
```

While for a personal account, something like this might do:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: '0 0 0 * *'  # the first day of the month

jobs:
  delete-old-container-images:
    - name: Delete old images
      uses: sondrelg/container-retention-policy@v0.1
      with:
        image-names: dev
        cut-off: One month ago UTC
        account-type: personal
        token: ${{ secrets.PAT }}
```

# Parameters

## image-names

* **Required**: `Yes`
* **Example**: `image-names: image1,image2,image3`

The names of the container images you want to delete old versions for. Takes one or several container image names as a
comma separated list.

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

The account type of the account running the action. The account type determines which API endpoints to use in the Github
API.

## org-name

* **Required**: `Only if account type is org`
* **Example**: `org-name: google`

The name of your organization.

## token

* **Required**: `Yes`
* **Example**: `token: ${{ secrets.PAT }}`

For the token, you need to pass a [personal access token](https://docs.github.com/en/github/authenticating-to-github/keeping-your-account-and-data-secure/creating-a-personal-access-token)
with access to the container registry. Specifically, you need to grant
it the following scopes:

- `read:packages`, and
- `delete:packages`

# Nice to knows

* The Github API restricts ut to fetching 100 image versions per image name,
so if your registry isn't 100% clean after the first job, don't be alarmed.

* If you accidentally delete something you shouldn't have, Github apparently has a
30 day grace period before actually deleting your image version. See [these docs](https://docs.github.com/en/rest/reference/packages#restore-package-version-for-an-organization)
for the information you need to restore your data.

# Contributing

Please do 👏
