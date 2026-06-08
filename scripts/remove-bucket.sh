#!/bin/sh
#
# Remove a rustfs/S3 bucket using credentials from vivo secrets.
# Usage: sh scripts/remove-bucket.sh [https://host/bucket]

if test $# -ge 1; then
    bucket_url="$1"
else
    printf "Bucket URL (e.g. https://rustfs.host/bucket): "
    read -r bucket_url
fi

printf "Credential profile (from vivo secrets): "
read -r profile

# Parse URL into scheme, host, bucket using parameter expansion
scheme="${bucket_url%%://*}"
rest="${bucket_url#*://}"
host="${rest%%/*}"
bucket="${rest#*/}"

if test -z "$host" || test -z "$bucket"; then
    printf "error: could not parse host and bucket from '%s'\n" "$bucket_url" >&2
    exit 1
fi

secrets_path="${VIVO_BACKUP_SECRETS:-$HOME/.config/vivo/secrets.yaml}"

creds=$(sops -d "$secrets_path" | yq ".credentials.$profile | .AWS_ACCESS_KEY_ID + \":\" + .AWS_SECRET_ACCESS_KEY")

if test -z "$creds"; then
    printf "error: no credentials found for profile '%s' in %s\n" "$profile" "$secrets_path" >&2
    exit 1
fi

printf "Removing %s://%s/%s ...\n" "$scheme" "$host" "$bucket"
env "MC_HOST_vivo-remove=$scheme://$creds@$host" mc rb "vivo-remove/$bucket" --force
