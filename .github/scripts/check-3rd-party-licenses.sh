#!/bin/bash
deps_file=${DEPS_FILE:-"DEPS.txt"}
dash_jar=${DASH_JAR:-"/tmp/dash.jar"}
dash_summary=${DASH_SUMMARY:-"DASH_SUMMARY.txt"}
project=${PROJECT:-"automotive.uprotocol"}
token=$1

echo "creating 3rd party dependency list..."
cargo tree -e no-build,no-dev --prefix none --no-dedupe \
  | sed -n '2~1p' \
  | sort -u \
  | grep -v '^[[:space:]]*$' \
  | sed -E 's|([^ ]+) v([^ ]+).*|crate/cratesio/-/\1/\2|' \
  > "$deps_file"

if [[ ! -r "$dash_jar" ]]; then
  echo "Eclipse Dash JAR file [${dash_jar}] not found, downloading latest version from GitHub..."
  wget_bin=$(which wget)
  if [[ -z "$wget_bin" ]]; then
    echo "wget command not available on path"
    exit 127
  else
    wget --quiet -O "$dash_jar" "https://repo.eclipse.org/service/local/artifact/maven/redirect?r=dash-licenses&g=org.eclipse.dash&a=org.eclipse.dash.licenses&v=LATEST"
  echo "successfully downloaded Eclipse Dash JAR to ${dash_jar}"
  fi
fi

if [[ -n "$token" ]]; then
  additional_args="-review -token $token -project $project"
else
  additional_args=""
fi
echo "checking 3rd party licenses..."
java -jar "$dash_jar" -timeout 60 -batch 90 -summary "$dash_summary" $additional_args "$deps_file"
