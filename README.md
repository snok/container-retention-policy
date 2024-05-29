[![release](https://img.shields.io/github/v/release/snok/container-retention-policy)](https://github.com/snok/container-retention-policy/releases/latest)

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

To use the action, simply add it to your GitHub workflow, like this. For the first run, we recommend running the action with `dry-run: true`.

```yaml
- uses: snok/container-retention-policy@v3
  name: Delete old test images
  with:
    account: snok
    token: ${{ secrets.PAT }}
    image-names: container-retention-policy
    image-tags: test* !v*
    tag-selection: untagged
    cut-off: 2w 1h 2m 1s
    dry-run: true
```

For a personal account, just replace the `snok` account name with the string "user".

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
    package1=$(docker manifest inspect ghcr.io/package1 | jq -r '.manifests.[] | .digest' | paste -s -d ', ' -)
    package2=$(docker manifest inspect ghcr.io/package2 | jq -r '.manifests.[] | .digest' | paste -s -d ', ' -)
    echo "multi-arch-digests=$package1,$package2" >> $GITHUB_OUTPUT

- uses: snok/container-retention-policy
  with:
    ...
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

# Things to be aware of

# Development

We use the [GitHub API](https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#list-packages-for-an-organization) to fetch data.

Fine grained tokens are not supported. See https://github.com/github/roadmap/issues/558.

## Rate limits

The documentation for GitHub rate limits can be found [here](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28).

The primary request limit for users authenticated with personal access tokens is 5000 requests per hour. The primary request limit for users authenticated with the built-in `GITHUB_TOKEN` is 1000 requests per repository, per hour. Limits also vary by account types, so it's best to use the response headers to know how many requests can safely be sent.

In addition to the primary rate limit, there are multiple secondary rate limits;

- No more than 100 concurrent requests
- No more than 900 points per endpoint, per minute, where a `GET` request is 1 point and a `DELETE` request is 5 ([source](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#calculating-points-for-the-secondary-rate-limit)).
- No more than 90 seconds of CPU time per 60 seconds of real time. No real way of knowing what time you've used is provided by GitHub - instead they suggest counting total response times.
- ~~No more than 80 content-creating requests per minute, and no more than 500 content-creating requests per hour~~



All but the last secondary limit might affect us, and the secondary rate limits are subject to change without notice. For this reason, it's recommended to use the returned `x-ratelimit-*` headers to know how to proceed ([source](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#checking-the-status-of-your-rate-limit)).

## Pagination

## Tokens

We can use three types of tokens:

- `$GITHUB_TOKEN` for
https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats/


- [ ] Multi-platform support https://github.com/Chizkiyahu/delete-untagged-ghcr-action/pull/16/files
- Oauth app authorization: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-appsjus

Discrete features:
- [x] Percent-url encoding
- [x] Keeping at least `n` images
- [x] Get package
- [x] Get package version
- [x] Delete package version
- [x] Handling primary rate limit
- [x] Handling secondary rate limits: concurrency
- [x] Handling secondary rate limits: tbd
- [x] Oauth token support
- [x] PAT support
- [x] Github token support
- [x] Parsing inputs as comma separated or space separate values (adjust data in action.yml?)
- [ ] Rely on pre-built docker image rather than building image
- [ ] Handling pagination
- [ ] Know which scopes are necessary for **each** type of token, and validate the scopes on the first request
- [ ] Graceful handling and logging of failed rate limit
- [ ] Graceful handling and logging of package versions that can't be downloaded because they have too many downloads
- [ ] Action outputs
- [ ] Support deleting multi-arch images
- [ ] Check that the `buffer` does not impact the concurrency limit, and write down what it actually does
- [ ] Check the wildmatch serde feature
- [ ] SafeStr for token vale
- [ ] Test Github app token
