#!/bin/sh
# Shell helpers of "Install Asset Hub" (scripts/installer.applescript). make-web-installer.mjs copies this
# file into the installer app; scripts/installer-helpers.test.sh runs the same functions without any UI.

# The CFBundleShortVersionString of an app folder; empty when the app or its version is missing.
app_version() {
	/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$1/Contents/Info.plist" 2>/dev/null
}

# Succeeds when "$1" is an app whose major version is below 2 (a 1.x copy). An app whose version
# cannot be read as a number is left alone.
is_legacy_app() {
	version=$(app_version "$1")
	[ -n "$version" ] || return 1
	major=${version%%.*}
	case $major in
	'' | *[!0-9]*) return 1 ;;
	esac
	[ "$major" -lt 2 ]
}

# Succeeds only when the SHA-256 of file "$1" is "$2" (an empty expected value never matches).
sha256_matches() {
	[ -n "$2" ] || return 1
	actual=$(shasum -a 256 "$1" | awk '{print $1}')
	[ "$actual" = "$2" ]
}
