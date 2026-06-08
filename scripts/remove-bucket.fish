#!/usr/bin/env fish
#
# Remove a rustfs/S3 bucket using credentials from vivo secrets.
# Usage: fish scripts/remove-bucket.fish [https://host/bucket]

if test (count $argv) -ge 1
    set bucket_url $argv[1]
else
    read -P "Bucket URL (e.g. https://rustfs.host/bucket): " bucket_url
end

read -P "Credential profile (from vivo secrets): " profile

# Parse endpoint and bucket from URL
# e.g. https://rustfs.cinnamon-trout.ts.net/filecabinet
#   → scheme:   https
#   → host:     rustfs.cinnamon-trout.ts.net
#   → bucket:   filecabinet
set parts (string split "/" $bucket_url)
set scheme (string replace ":" "" $parts[1])
set host $parts[3]
set bucket $parts[4]

if test -z "$host" -o -z "$bucket"
    echo "error: could not parse host and bucket from '$bucket_url'" >&2
    exit 1
end

set secrets_path (test -n "$VIVO_BACKUP_SECRETS" && echo $VIVO_BACKUP_SECRETS || echo ~/.config/vivo/secrets.yaml)

set creds (sops -d $secrets_path | yq ".credentials.$profile | .AWS_ACCESS_KEY_ID + \":\" + .AWS_SECRET_ACCESS_KEY")

if test -z "$creds"
    echo "error: no credentials found for profile '$profile' in $secrets_path" >&2
    exit 1
end

echo "Removing $scheme://$host/$bucket ..."
env MC_HOST_vivo-remove="$scheme://$creds@$host" mc rb vivo-remove/$bucket --force
