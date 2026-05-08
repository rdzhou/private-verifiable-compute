# Deploy PVC on Confidential Containers

PVC can be deployed on a Kubernetes cluster that already has Confidential Containers and Kata configured.

## Prerequisite

This walkthrough assumes:
- your Kubernetes cluster is already available
- Kata is already configured as a runtime on the cluster
- the runtime classes `kata-qemu-snp` and `kata-qemu-nvidia-gpu-snp` already exist
- the storage class `longhorn` already exists

Those names come from `deployment/envs/kata.yaml`, which is used by the current Kata deployment flow.

## Tools

- [Kubectl](https://kubernetes.io/docs/tasks/tools/) to access the cluster
- [Helm](https://helm.sh/docs/intro/install/) to deploy PVC
- [Gcloud CLI](https://cloud.google.com/sdk/docs/install) to authenticate with Artifact Registry
- [Bazel](https://bazel.build/install) to build and push images

## Prepare env variables

Fill in the correct values relevant to your project in the `.env` file.

```
cp env.example .env
```

- `project_id` The unique identifier for your project across Google Cloud.

## Push Images

```shell
gcloud auth configure-docker us-docker.pkg.dev
source .env
bazel run //:push_all_images --action_env=namespace=<namespace-to-deploy> --action_env=project_id=$project_id
```

> [!IMPORTANT]
> The `--action_env=namespace=<namespace-to-deploy>` and `--action_env=project_id=$project_id` flags are required.

## Deploy

Deploy PVC with the Kata platform.

```shell
./deployment/deploy.sh --platform=kata --namespace=<namespace-to-deploy>
```

To preview the rendered release without applying changes:

```shell
./deployment/deploy.sh --platform=kata --namespace=<namespace-to-deploy> --dry-run
```

`--dry-run` does not apply changes, but it still talks to Helm and the current cluster context.

## Check The Result

```shell
kubectl get pods -n <namespace-to-deploy>
kubectl get svc -n <namespace-to-deploy>
kubectl port-forward -n <namespace-to-deploy> --address 0.0.0.0 svc/pvc-client 8083:8083
```

Open `localhost:8083` in your browser to access the client application.

## Clean Up Deployment

```shell
helm delete private-verifiable-compute -n <namespace-to-deploy>
```
