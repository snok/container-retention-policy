# Creating a release

To create a release we need to:

1. Manually trigger the [deploy](.github/workflows/deploy.yaml) workflow to build new images
2. Update the image tag in the [action.yaml](action.yaml)
3. Push the change and create a GitHub release post for the repo
