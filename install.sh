#!/bin/sh
set -e

if [ -n "$PREFIX" ] && echo "$PREFIX" | grep -q "com.termux"; then
    INSTALLER_URL="https://github.com/Spectra010s/fsmirror/releases/latest/download/fsmirror-android-installer.sh"
else
    INSTALLER_URL="https://github.com/Spectra010s/fsmirror/releases/latest/download/fsmirror-installer.sh"
fi

curl -fsSL "$INSTALLER_URL" | sh
