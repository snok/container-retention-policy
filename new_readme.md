This is a [container github action](https://docs.github.com/en/actions/creating-actions/creating-a-docker-container-action).


TODO: WHat about reuploads? Can we use created-at?

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
- [ ] Get package
- [ ] Get package version
- [ ] Delete package version
- [ ] Handling primary rate limit
- [ ] Handling secondary rate limits: concurrency
- [ ] Handling secondary rate limits: tbd
- [ ] Oauth token support
- [ ] PAT support
- [ ] Github token support
- [ ] Parsing inputs as comma separated or space separate values (adjust data in action.yml?)
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

## Safely handling multi-platform (multi-arch) packages

Multi-platform packages can break if not properly handled. If you host multi-platform packages, make sure to read and implement the fix explained below.

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

If we delete some of these, we'll either delete some platform targets, or the image manifests, which will produce errors when trying to pull the image in general.

### The solution

While the API does not provide enough metadata, the docker-cli does. If we use `docker manifest inspect ghcr.io/snok/container-retention-policy:multi-arch`, we'll get:

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

This means, to make sure your multi-platform packages aren't broken, you should do the following, in your workflow:

```yaml
- name: Login to GitHub Container Registry
  uses: docker/login-action@v3
  with:
    registry: ghcr.io
    username: ${{ github.actor }}
    password: ${{ secrets.GITHUB_TOKEN }}

- name: Get multi-platform package version SHAs
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

By passing the SHAs of the manifests and specific platform targets to the action, we are able to not delete them.


TODO: Make sure that the echo >>GITHUB_OUTPUT actually works when running the action as a container
