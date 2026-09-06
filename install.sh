#!/usr/bin/env bash

# Script to download and install ⎓⎓dcdc, https://github.com/cybtachyon/dcdc
# Usage: install.sh or install.sh <version>

set -o errexit
set -o pipefail
set -o nounset

if [ ! -d /usr/local/bin ]; then echo 'Using sudo to create missing /usr/local/bin' && sudo mkdir -p /usr/local/bin; fi

DCDC_GITHUB_OWNER=${DCDC_GITHUB_OWNER:-cybtachyon}
ARTIFACTS="dcdc dcdc-hostname mkcert"

TMPDIR=/tmp

RED='\033[31m'
GREEN='\033[32m'
YELLOW='\033[33m'
RESET='\033[0m'
OS=$(uname)
BIN_OWNER=$(ls -ld /usr/local/bin | awk '{print $3}')
USER=$(whoami)
SHA_CMD=""
FILE_BASE_NAME=""

# @see https://gist.github.com/Ariel-Rodriguez/9e3c2163f4644d7a389759b224bfe7f3
semver_compare() {
  local version_a version_b pr_a pr_b
  # strip word "v" and extract first subset version (x.y.z from x.y.z-foo.n)
  version_a=$(echo "${1//v/}" | awk -F'-' '{print $1}')
  version_b=$(echo "${2//v/}" | awk -F'-' '{print $1}')

  if [ "$version_a" \= "$version_b" ]
  then
    # check for pre-release
    # extract pre-release (-foo.n from x.y.z-foo.n)
    pr_a=$(echo "$1" | awk -F'-' '{print $2}')
    pr_b=$(echo "$2" | awk -F'-' '{print $2}')

    ####
    # Return 0 when A is equal to B
    [ "$pr_a" \= "$pr_b" ] && echo 0 && return 0

    ####
    # Return 1

    # Case when A is not pre-release
    if [ -z "$pr_a" ]
    then
      echo 1 && return 0
    fi

    ####
    # Case when pre-release A exists and is greater than B's pre-release

    # extract numbers -rc.x --> x
    number_a="${pr_a//[!0-9]/}"
    number_b="${pr_b//[!0-9]/}"
    [ -z "${number_a}" ] && number_a=0
    [ -z "${number_b}" ] && number_b=0
    [ "$pr_a" \> "$pr_b" ] && [ -n "$pr_b" ] && echo 1 && return 0

    ####
    # Return -1 when A is lower than B
    echo -1 && return 0
  fi
  arr_version_a=("${version_a//./ }")
  arr_version_b=("${version_b//./ }")
  cursor=0
  # Iterate arrays from left to right and find the first difference
  while [ "$([ "${arr_version_a[$cursor]}" -eq "${arr_version_b[$cursor]}" ] && [ $cursor -lt ${#arr_version_a[@]} ] && echo true)" == true ]
  do
    cursor=$((cursor+1))
  done
  [ "${arr_version_a[$cursor]}" -gt "${arr_version_b[$cursor]}" ] && echo 1 || echo -1
}

if [[ $EUID -eq 0 ]]; then
  echo "This script must NOT be run with sudo/root. Please re-run without sudo." 1>&2
  exit 102
fi

unamearch=$(uname -m)
case ${unamearch} in
  x86_64) ARCH="amd64";
  ;;
  aarch64) ARCH="arm64";
  ;;
  arm64) ARCH="arm64"
  ;;
  *) printf "%s" "${RED}Sorry, your machine architecture ${unamearch} is not currently supported.\n${RESET}" && exit 106
  ;;
esac

LATEST_RELEASE=$(curl -fsSL -H 'Accept: application/json' "https://github.com/${DCDC_GITHUB_OWNER}/dcdc/releases/latest" || (printf "${RED}Failed to get find latest release${RESET}\n" >/dev/stderr && exit 107))
# The releases are returned in the format {"id":3622206,"tag_name":"hello-1.0.0.11",...}, we have to extract the tag_name.
LATEST_VERSION=$(echo ${LATEST_RELEASE} | sed -e 's/.*"tag_name":"\([^"]*\)".*/\1/')

VERSION=$LATEST_VERSION
if [ $# -ge 1 ]; then
  VERSION=$1
fi
RELEASE_BASE_URL="https://github.com/${DCDC_GITHUB_OWNER}/dcdc/releases/download/${VERSION}"

if [[ "$OS" == "Darwin" ]]; then
    SHA_CMD="shasum -a 256 --ignore-missing"
    FILE_BASE_NAME="dcdc_macos"
elif [[ "$OS" == "Linux" ]]; then
    SHA_CMD="sha256sum --ignore-missing"
    FILE_BASE_NAME="dcdc_linux"
else
    printf "%s" "${RED}Sorry, this installer does not support your platform at this time.${RESET}\n"
    exit 1
fi


FILE_BASE_NAME="${FILE_BASE_NAME}-${ARCH}"

if ! docker --version >/dev/null 2>&1; then
    printf "%s" "${YELLOW}A docker provider is required for dcdc. Please see https://docs.dcdc.com/en/stable/users/install/docker-installation/.${RESET}\n"
fi

TARBALL="${FILE_BASE_NAME}.${VERSION}.tar.gz"
OLD_CHECKSUM=$(semver_compare "${VERSION}" "v1.19.3")
SHAFILE=checksums.txt
if [ "${OLD_CHECKSUM}" != 1 ]; then
  SHAFILE="${TARBALL}.sha256.txt"
fi

curl -fsSL "${RELEASE_BASE_URL}/${TARBALL}" -o "${TMPDIR}/${TARBALL}" || (printf "%s" "${RED}Failed downloading $RELEASE_BASE_URL/$TARBALL${RESET}\n" && exit 108)
curl -fsSL "${RELEASE_BASE_URL}/${SHAFILE}" -o "${TMPDIR}/${SHAFILE}" || (printf "%s" "${RED}Failed downloading $RELEASE_BASE_URL/$SHAFILE${RESET}\n" && exit 109)

cd "${TMPDIR}"
$SHA_CMD -c "${SHAFILE}"
tar -xzf "${TARBALL}"

printf "%s" "${GREEN}Download verified. Ready to place dcdc, dcdc-hostname, and mkcert in your /usr/local/bin.${RESET}\n"

if command -v brew >/dev/null && brew info dcdc >/dev/null 2>/dev/null ; then
  echo "Attempting to unlink any homebrew-installed dcdc with 'brew unlink dcdc'"
  brew unlink dcdc >/dev/null 2>&1 || true
fi
if [ -L /usr/local/bin/dcdc ] ; then
    printf "%s" "${RED}dcdc already exists as a link in /usr/local/bin/dcdc. Was it installed with homebrew?${RESET}\n"
    printf "%s" "${RED}Cowardly refusing to install over existing symlink${RESET}\n"
    exit 101
fi

SUDO=""
if [[ "${BIN_OWNER}" != "${USER}" ]]; then
  SUDO=sudo
fi
if [ ! -z "${SUDO}" ]; then
    printf "%s" "${YELLOW}Running \"sudo mv -f ${ARTIFACTS} /usr/local/bin/\" Please enter your password if prompted.${RESET}\n"
fi
for item in ${ARTIFACTS}; do
  if [ -f "${item}" ]; then
    chmod +x "${item}"
    ${SUDO} mv -f "${item}" /usr/local/bin/
  fi
done

if command -v brew >/dev/null ; then
    if [ -d "$(brew --prefix)/etc/bash_completion.d" ]; then
        bash_completion_dir=$(brew --prefix)/etc/bash_completion.d
        cp dcdc_bash_completion.sh "${bash_completion_dir}/dcdc"
        printf "%s" "${GREEN}Installed dcdc bash completions in $bash_completion_dir${RESET}\n"
        rm -f dcdc_bash_completion.sh
    else
        printf "%s" "${YELLOW}Bash completion for dcdc was not installed. You may manually install /tmp/dcdc_bash_completion.sh in your bash_completion.d directory.${RESET}\n"
    fi

    if  [ -d "$(brew --prefix)/share/zsh-completions" ] && [ -f dcdc_zsh_completion.sh ]; then
        zsh_completion_dir=$(brew --prefix)/share/zsh-completions
        cp dcdc_zsh_completion.sh "${zsh_completion_dir}/_dcdc"
        printf "%s" "${GREEN}Installed dcdc zsh completions in ${zsh_completion_dir}${RESET}\n"
        rm -f dcdc_zsh_completion.sh
    else
        printf "%s" "${YELLOW}zsh completion for dcdc was not installed. You may manually install ${TMPDIR}/dcdc_zsh_completion.sh in your zsh-completions directory.${RESET}\n"
    fi
fi

rm -f "${TMPDIR}${TARBALL}" "${TMPDIR}/$SHAFILE"

if command -v mkcert >/dev/null; then
  printf "%s" "${YELLOW}Running mkcert -install, which may request your sudo password.'.${RESET}\n"
  mkcert -install
fi

hash -r

printf "%s" "${GREEN}dcdc is now installed. Run \"dcdc\" and \"dcdc --version\" to verify your installation and see usage.${RESET}\n"
