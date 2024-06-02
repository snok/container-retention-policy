[![release](https://img.shields.io/github/v/release/snok/container-retention-policy)](https://github.com/snok/container-retention-policy/releases/latest)

# 📘 GHCR Container Retention Policy

A GitHub action for deleting old image versions from the GitHub container registry.

Storage isn't free and registries can often get bloated with unused images. Having a retention policy to prevent clutter
makes sense in most cases.

- ✅ Supports both organizational and personal accounts
- 👮 Supports several token types for authentication
- 🌱 The docker image used is only 2.4Mi, and the total runtime is a few seconds

# Content

- [Usage](#usage)
- [Parameters](#parameters)
- [Examples](#examples)
- [Nice to knows](#nice-to-knows)
- [Contributing](#contributing)

# Usage

To use the action, simply add it to your GitHub workflow like this. For your first run, we recommend running the action with `dry-run: true`.

```yaml
- uses: snok/container-retention-policy@v3
  name: Delete old test images
  with:
    account: snok
    token: ${{ secrets.PAT }}
    image-names: container-retention-policy
    image-tags: test* dev* # target any image that has a tag starting with the word test or dev
    tag-selection: both  # delete both tagged and untagged (replaced) images
    cut-off: 2w 3d
    dry-run: true
```

For a personal account, just replace the `snok` account name with the string "user".

To trigger the workflow, a [scheduled event](https://docs.github.com/en/actions/reference/events-that-trigger-workflows#schedule),
probably makes sense most of the time.

# Parameters

### Account

* **Required**: `Yes`
* **Example**: `account: acme` for an organization named "acme" or `account: user` for personal accounts

The account field provides the action with information on whether the workflow is run by an organization
or a user (each have different API endpoints in GitHub's package APIs) - and if it's run by an organization it
also gives us the organization name, as this is needed for calling the org. API endpoints.

### Token

* **Required**: `Yes`
* **Example**: `token: ${{ secrets.PAT }}`, `token: ${{ secrets.GITHUB_TOKEN }}`, or `token: $OAUTH_TOKEN`

The token is used to authenticate the action when making API calls to the package APIs.

#### Temporal tokens

If you're using a temporal token (`${{ secrets.GITHUB_TOKEN }}`), you should note that the filtering operators
described for `image-names` below *can not be used*. Temporal tokens are not usable for the [list-packages endpoint](https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#list-packages-for-an-organization), so we have
to work around it by calling the [get-package endpoint](https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#get-a-package-for-an-organization) instead. This means `image-names` needs to contain
exact names that we can use when constructing the endpoint URLs.

For a temporal token to work, it is necessary for the repository running the workflow to have the `Admin` role
assigned in the package settings.

#### Classic personal access tokens

Personal access tokens must have the `packages:write` scopes.

### GitHub app tokens

Github app tokens must have the `packages:write` scopes.

To fetch an app token, you can structure your workflow like this:

```yaml
- name: Generate a token
  id: generate-token
  uses: actions/create-github-app-token@v1
  with:
    app-id: ${{ secrets.GH_APP_ID }}
    private-key: ${{ secrets.GH_APP_PRIVATE_KEY }}

- uses: snok/container-retention-policy@v3
  with:
    account: snok
    token: ${{ steps.generate-token.outputs.token }}
```

### Cut-off

* **Required**: `Yes`
* **Example**: `cut-off: 1w` or `cut-off: 5h 2s`

Specifies how old package versions need to be before being considered for deletion.

The cut-off value parsing is handled by the [humantime](https://crates.io/crates/humantime) Rust crate.
Please take a look at their documentation if you have trouble getting it to work.

## image-names

* **Required**: `Yes`
* **Examples**:
  * `image-names: container-retention-policy` to select the `container-retention-policy` image
  * `image-names: dev* test*` to select any image starting with the string `dev` or `test`
  * `image-names: !v*` to select any image *not* starting with the string `v`

The name(s) of the container image(s) you want to delete package versions for. Supports filtering
with `*` and `!`, where `*` functions as a wildcard and `!` means to not select image names
matching the remaining expression. These operators are not available if you're using a temporal token
for the `token` input. See the `token` parameter section for more info.

### image-tags

* **Required**: `No`
* **Example**: `image-tags: !latest`

Optionally narrows the selection of package versions based on associated tags. Works the same way as the
`image-names` parameter. See above for info on supported syntax.

### skip-shas

* **Required**: `No`
* **Example**: `skip-shas: sha256:610a8286bda2dcc713754078070341b8e696be0b02c0e36b2d48f1447c7162af,sha256:a1b6216dfcb74a02b33a2ed68b5dc9c1bd6aa1552d1e377155e8cb348525c533`

Optionally protects specific package versions by their digest. This parameter was added to support
proper handling of multi-platform images. See [safely handling multi-platform (multi-arch) packages](#safely-handling-multi-platform-multi-arch-packages) for details.

### tag-selection

* **Required**: `No`
* **Example**: `tag-selection: both` or `tag-selection: untagged` or `tag-selection: tagged`
* **Default**: `tag-selection: both`

Optionally lets you select only tagged or untagged images. Both are selected by default.

### keep-at-least

* **Required**: `No`
* **Example**: `keep-at-least: 5`
* **Default**: `keep-at-least: 0`

How many images to keep, out of the most recent tagged images selected. If there are 10 tagged package
versions selected, after filtering on names, tags, and cut-off, and there's a keep-at-least count of 3 set,
then we will retain the 3 most recently created package versions and delete 7.

This parameter will not prevent deletion of untagged images, because we do not know of a valid use-case for this behavior.

The parameter can be useful, e.g., to protect some number of tagged images, so that rollbacks don't fail
in Kubernetes. See [making sure there are enough revisions available for rollbacks in Kubernetes](#making-sure-there-are-enough-revisions-available-for-rollbacks-in-kubernetes) for details.

### timestamp-to-use

* **Required**: `No`
* **Example**: `timestamp-to-use: created_at`
* **Default**: `timestamp-to-use: updated_at`

Whether we should use the `created_at` or `updated_at` timestamp when filtering based on the `cut-off` parameter.

### dry-run

* **Required**: `No`
* **Example**: `dry-run: true`
* **Default**: `dry-run: false`

When `true` we will output which package versions would have been deleted, but not actually delete them.


### rust-log

* **Required**: `No`
* **Examples**:
  * `rust-log: container_retention_policy=error`
  * `rust-log: info`
  * `rust-log: container_retention_policy=debug,hyper_util=info`
* **Default**: `container_retention_policy=info`

Configures the log-levels of the action.

The action uses the [env_logger](https://docs.rs/env_logger/latest/env_logger/) crate to set log-levels,
and any expression supported by `env_logger` will work. We recommend running the action with debug logs
if you experience any weird behavior.

# Examples

## Organization

```yaml
name: Delete old container images

on:
  schedule:
    - cron: "0 0 * * *"  # every day at midnight

jobs:
  delete-package-versions:
    name: Delete package versions older than 4 weeks
    runs-on: ubuntu-latest
    steps:
      - uses: snok/container-retention-policy@v3
        with:
          account: snok
          token: ${{ secrets.PAT }}
          image-names: foo bar baz  # select three packages
          image-tags: !prod !qa  # protect package versions tagged with 'prod' or 'qa'
          tag-selection: both  # select both tagged and untagged package versions
          cut-off: 4w  # package versions should be older than 4 weeks
          dry-run: false
```

## Personal account

```yaml
name: Delete old container images

on:
  schedule:
    - cron: "0 0 * * *"  # every day at midnight

jobs:
  delete-package-versions:
    name: Delete all untagged package versions
    runs-on: ubuntu-latest
    steps:
      - uses: snok/container-retention-policy@v3
        with:
          account: user
          token: ${{ secrets.PAT }}
          image-names: *  # all packages owned by the account
          tag-selection: untagged
          cut-off: 1h
```

# Outputs

## deleted

Comma-separated list of `image-name:version-id` for each image deleted.

## failed

Comma-separated list of images that we weren't able to delete. Check
logs for responses.

# Nice to knows

## Supported operating systems

GitHub actions running containers (like this one) are currently only supported
by ubuntu-runners. This is a GitHub action limitation.

## Running the application outside the action

The action is a Rust application packaged as a container, so if you prefer
to run the program elsewhere you may:

- Pull the docker image and run it with:

  ```
  docker run \
            -e RUST_LOG=container_retention_policy=info \
            ghcr.io/snok/container-retention-policy:v3.0.0-alpha2  \
            --account snok \
            --token $PAT \
            --cut-off 1d \
            --image-names "container-retention-policy*"
  ```

- Clone the repo, compile it, and run the binary directly:

  ```
  git clone git@github.com:snok/container-retention-policy.git
  cargo build --release
  RUST_LOG=container_retention_policy=info \
    ./target/releases/container-retention-policy \
     --account snok \
     --token $PAT \
     --cut-off 1d \
     --image-names "container-retention-policy*"
  ```

## Making sure there are enough revisions available for rollbacks in Kubernetes

If you're deploying containers to Kubernetes, one thing to beware of is not to specify retention policies that prevent you from rolling back deployments, should you need to.

If you're following best-practices for tagging your container images, you'll likely be tagging them with versions, dates, or some other moving tag strategy. In this case, it can be hard to protect *some* package versions from being deleted by using the `image-tags` filters. Instead, you can use the `keep-at-least` argument, which will retain `n` package versions per package specified:

```yaml
name: Delete old container images

on:
  schedule:
    - cron: "0 0 * * *"  # every day at midnight

jobs:
  delete-package-versions:
    name: Delete package versions older than 4 weeks, but keep the latest 5 in case of rollbacks
    runs-on: ubuntu-latest
    steps:
      - uses: snok/container-retention-policy@v3
        with:
          account: snok
          token: ${{ secrets.PAT }}
          image-names: foo bar baz  # select three packages
          image-tags: *  # any image tag
          tag-selection: both  # select both tagged and untagged package versions
          cut-off: 1w  # package versions should be older than 4 weeks
          keep-at-least: 5  # keep up to `n` tagged package versions for each of the packages
```

The action will prioritize keeping newer package versions over older ones.

## Safely handling multi-platform (multi-arch) packages

This action (or rather, naïve deletion of package version in GitHub's container registry) can break your multi-platform packages, if deletion is not handled carefully. If you're hosting multi-platform packages, please implement the action as described below.

### The problem

GitHub's container registry supports uploads of multi-platform  packages, with commands like:

```
docker buildx build \
    -t ghcr.io/snok/container-retention-policy:multi-arch \
    --platform linux/amd64,linux/arm64 . \
    --push
```

However, they do not provide enough metadata on in the packages API to properly handle deletion for multi-platform packages. From the build above, the API will return 5 package versions. From these five, one package version contains our `multi-arch` tag, and four are untagged, with no references to each-other:

```json
[
  {
    "id": 214880827,
    "name": "sha256:e8530d7d4c44954276715032c027882a2569318bb7f79c5a4fce6c80c0c1018e",
    "created_at": "2024-05-11T12:42:55Z",
    "metadata": {
      "package_type": "container",
      "container": {
        "tags": [
          "multi-arch"
        ]
      }
    }
  },
  {
    "id": 214880825,
    "name": "sha256:ca5bf1eaa2a393f30d079e8fa005c73318829251613a359d6972bbae90b491fe",
    "created_at": "2024-05-11T12:42:54Z",
    "metadata": {
      "package_type": "container",
      "container": {
        "tags": []
      }
    }
  },
  {
    "id": 214880822,
    "name": "sha256:6cff2700a9a29ace200788b556210973bd35a541166e6c8a682421adb0b6e7bb",
    "created_at": "2024-05-11T12:42:54Z",
    "metadata": {
      "package_type": "container",
      "container": {
        "tags": []
      }
    }
  },
  {
    "id": 214880821,
    "name": "sha256:f8bc799ae7b6ba95595c32e12075d21328dac783c9c0304cf80c61d41025aeb2",
    "created_at": "2024-05-11T12:42:53Z",
    "html_url": "https://github.com/orgs/snok/packages/container/container-retention-policy/214880821",
    "metadata": {
      "package_type": "container",
      "container": {
        "tags": []
      }
    }
  },
  {
    "id": 214880818,
    "name": "sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03",
    "created_at": "2024-05-11T12:42:53Z",
    "html_url": "https://github.com/orgs/snok/packages/container/container-retention-policy/214880818",
    "metadata": {
      "package_type": "container",
      "container": {
        "tags": []
      }
    }
  }
]
```

If we delete some of these, we'll either delete some of the platform targets, or the underlying image manifests, which will produce errors when trying to pull any image.

### The solution

While GitHub's packages API does not provide enough metadata for us to adequately handle this, the docker-cli does. If we use `docker manifest inspect ghcr.io/snok/container-retention-policy:multi-arch`, we'll see:

```json
{
   "schemaVersion": 2,
   "mediaType": "application/vnd.oci.image.index.v1+json",
   "manifests": [
      {
         "mediaType": "application/vnd.oci.image.manifest.v1+json",
         "size": 754,
         "digest": "sha256:f8bc799ae7b6ba95595c32e12075d21328dac783c9c0304cf80c61d41025aeb2",
         "platform": {
            "architecture": "amd64",
            "os": "linux"
         }
      },
      {
         "mediaType": "application/vnd.oci.image.manifest.v1+json",
         "size": 754,
         "digest": "sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03",
         "platform": {
            "architecture": "arm64",
            "os": "linux"
         }
      },
      {
         "mediaType": "application/vnd.oci.image.manifest.v1+json",
         "size": 567,
         "digest": "sha256:17152a70ea10de6ecd804fffed4b5ebd3abc638e8920efb6fab2993c5a77600a",
         "platform": {
            "architecture": "unknown",
            "os": "unknown"
         }
      },
      {
         "mediaType": "application/vnd.oci.image.manifest.v1+json",
         "size": 567,
         "digest": "sha256:86215617a0ea1f77e9f314b45ffd578020935996612fb497239509b151a6f1ba",
         "platform": {
            "architecture": "unknown",
            "os": "unknown"
         }
      }
   ]
}
```

Which lists all the SHAs of the images associated with this tag.

This means, you can do the following when implementing this action, to protect against partial deletion of your multi-platform images:

```yaml
- name: Login to GitHub Container Registry
  uses: docker/login-action@v3
  with:
    registry: ghcr.io
    username: ${{ github.actor }}
    password: ${{ secrets.GITHUB_TOKEN }}

- name: Fetch multi-platform package version SHAs
  id: multi-arch-shas
  run: |
    package1=$(docker manifest inspect ghcr.io/package1 | jq -r '.manifests.[] | .digest' | paste -s -d ' ' -)
    package2=$(docker manifest inspect ghcr.io/package2 | jq -r '.manifests.[] | .digest' | paste -s -d ' ' -)
    echo "multi-arch-digests=$package1,$package2" >> $GITHUB_OUTPUT

- uses: snok/container-retention-policy
  with:
    skip-shas: ${{ steps.multi-arch-digests.outputs.multi-arch-shas }}
```

In other words, you can fetch the manifest list, and pass it to this action as a comma-separated string of SHAs, to prevent give us the information we need to avoid deleting package versions which are used in multi-platform packages.

--------




This is a [container github action](https://docs.github.com/en/actions/creating-actions/creating-a-docker-container-action).


TODO: WHat about reuploads? Can we use created-at?

TODO: Specify that keep-at-least means to keep `n` package versions for each package, after it has satisfied filters. Explain that keeping `n` adhoc releases for rolling back kubernetes is a legitimate use case. Keeping the last 10 days of packages is not.

TODO: Ensure we sort keep-at-least by date and keep the most recent.

- [ ] Add note about keep_at_least keeping `n` number of package versions per image, and that it will prioritize keeping newer versions

- [ ] Add explanation of what an image version is

## Restoring a deleted image

If you accidentally delete something you shouldn't have, GitHub has a 30-day grace period before actually
deleting your image version. See [these docs](https://docs.github.com/en/rest/reference/packages#restore-package-version-for-an-organization) for details.

## Rate limits

The documentation for GitHub rate limits can be found [here](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28).

The primary request limit for users authenticated with personal access tokens is 5000 requests per hour. The primary request limit for users authenticated with the built-in `GITHUB_TOKEN` is 1000 requests per repository, per hour. Limits also vary by account types, so it's best to use the response headers to know how many requests can safely be sent.

In addition to the primary rate limit, there are multiple secondary rate limits;

- No more than 100 concurrent requests
- No more than 900 points per endpoint, per minute, where a `GET` request is 1 point and a `DELETE` request is 5 ([source](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#calculating-points-for-the-secondary-rate-limit)).
- No more than 90 seconds of CPU time per 60 seconds of real time. No real way of knowing what time you've used is provided by GitHub - instead they suggest counting total response times.
- ~~No more than 80 content-creating requests per minute, and no more than 500 content-creating requests per hour~~

All but the last secondary limit might are handled by the action. However, secondary rate limits are subject to
change without notice. If you run into problems, please open an issue.
