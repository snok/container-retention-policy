This is a [container github action](https://docs.github.com/en/actions/creating-actions/creating-a-docker-container-action).

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
