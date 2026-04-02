# Build PVC

Install depdencies.
```
./scripts/install_dev_dependencies.sh
```

Build the project with Bazel.
```
bazel build //...
```

Load images to local image repoisitory.
```
bazel run //:load_all_images
```

## Build inside a container

To make the build process more reproducible and streamline the dependency installation, we support building the project inside a container with the following scripts.

```
# Build the builder image
scripts/build_pvc_builder_image.sh
# Run the container to build the executables.
# The bazel cache is stored in ${REPO_ROOT}/.pvc-bazel-cache.
scripts/reproducible_build.sh
```
