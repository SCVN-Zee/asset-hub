-- "Install Asset Hub": the small installer in the web .dmg (bun run dist:web). It downloads the build of
-- one release from the public release repo, checks it against the SHA-256 baked in below, installs it
-- into Applications and opens it. Any Asset Hub 1.x it finds is moved to the Trash first. Downloaded with
-- curl, the app carries no quarantine flag, so macOS opens it without the "unidentified developer" step.
-- {{BASE_URL}}, {{VERSION}} and {{SHA256}} are filled in by the build script (make-web-installer.mjs).
-- The checks that need no dialog (version below 2, checksum) are in installer-helpers.sh, which the
-- build script copies into this app's Resources.

property baseURL : "{{BASE_URL}}"
property appVersion : "{{VERSION}}"
property expectedSHA : "{{SHA256}}"
property archiveName : "Asset.Hub.app.tar.gz"
property appName : "Asset Hub"

-- Asks the app at appPath to quit, when it is running (a quit sent to an app that is closed could open it).
on quitApp(appPath)
	try
		do shell script "pgrep -f " & quoted form of (appPath & "/Contents/MacOS/") & " > /dev/null"
	on error
		return
	end try
	do shell script "osascript -e " & quoted form of ("tell application \"" & appPath & "\" to quit") & " > /dev/null 2>&1; true"
	delay 1
end quitApp

-- Moves the 1.x app at appPath to the Trash through Finder (so it can be put back); leaves anything else.
on trashLegacyApp(helpers, appPath)
	try
		do shell script ". " & quoted form of helpers & "; is_legacy_app " & quoted form of appPath
	on error
		return
	end try
	my quitApp(appPath)
	tell application "Finder" to delete (POSIX file appPath as alias)
end trashLegacyApp

on run
	set work to ""
	try
		-- Built for Apple Silicon (M1–M4) only, and for macOS 14 or later.
		if (do shell script "uname -m") is not "arm64" then error "Asset Hub runs on Macs with Apple silicon (M1, M2, M3, M4). This Mac has an Intel chip."
		if (do shell script "sw_vers -productVersion | cut -d. -f1") as integer < 14 then error "Asset Hub needs macOS 14 (Sonoma) or later."
		set helpers to POSIX path of (path to resource "installer-helpers.sh")

		display dialog "This installer downloads Asset Hub " & appVersion & " (about 20 MB) from GitHub and installs it in your Applications folder. Asset Hub 1.x, if you have it, is moved to the Trash. Asset Hub 2 starts fresh: it does not use the data of 1.x." buttons {"Cancel", "Install"} default button "Install" cancel button "Cancel" with title "Install Asset Hub" with icon note

		set work to do shell script "mktemp -d -t assethub"

		-- Size first, for the progress bar; then the download runs in the background while it is watched.
		set progress total steps to 100
		set progress completed steps to 0
		set progress description to "Connecting to GitHub…"
		set sizeOf to " | awk 'tolower($1)==\"content-length:\" {n=$2} END {gsub(/\\r/,\"\",n); print n+0}'"
		set zipURL to baseURL & archiveName
		set total to 0
		try
			set total to (do shell script "curl -fsIL " & quoted form of zipURL & sizeOf) as integer
		end try
		if total is 0 then error "Asset Hub " & appVersion & " could not be found on GitHub. Check your internet connection and try again."
		set zipPath to work & "/" & archiveName
		-- A dropped connection is retried up to 3 times, resuming where it stopped (-C -). curl's own
		-- message goes to curl.log; the exit code is written aside and renamed into place, so "status"
		-- never exists half-written.
		set fetch to "for i in 1 2 3; do curl -fsSL -C - --connect-timeout 20 -o " & quoted form of zipPath & " " & quoted form of zipURL & " 2>> " & quoted form of (work & "/curl.log") & "; s=$?; [ $s = 0 ] && break; sleep 3; done; echo $s > " & quoted form of (work & "/status.part") & "; mv " & quoted form of (work & "/status.part") & " " & quoted form of (work & "/status")
		do shell script "(" & fetch & ") > /dev/null 2>&1 &"
		set progress description to "Downloading Asset Hub…"
		repeat
			delay 0.3
			set got to (do shell script "stat -f %z " & quoted form of zipPath & " 2>/dev/null || echo 0") as integer
			if total > 0 then set progress completed steps to (got * 100 div total)
			set progress additional description to (got div 1048576 as text) & " / " & (total div 1048576 as text) & " MB"
			if (do shell script "test -f " & quoted form of (work & "/status") & " && echo done || echo wait") is "done" then exit repeat
		end repeat
		set curlStatus to do shell script "cat " & quoted form of (work & "/status")
		if curlStatus is not "0" then
			set detail to do shell script "tail -n 1 " & quoted form of (work & "/curl.log") & " 2>/dev/null; true"
			error "Asset Hub could not be downloaded from GitHub (error " & curlStatus & "). Check your internet connection and try again." & return & return & detail
		end if

		-- Only a build that matches the checksum baked into this installer gets installed.
		set progress description to "Checking the download…"
		set progress additional description to ""
		try
			do shell script ". " & quoted form of helpers & "; sha256_matches " & quoted form of zipPath & " " & quoted form of expectedSHA
		on error
			error "The download does not match its checksum, so it was not installed. Please try again later."
		end try

		-- Unpacked aside first: the installed app is only touched once the new one is complete.
		set progress description to "Installing into Applications…"
		set unpacked to work & "/unpacked"
		do shell script "mkdir " & quoted form of unpacked & " && tar -xzf " & quoted form of zipPath & " -C " & quoted form of unpacked
		if (do shell script "test -d " & quoted form of (unpacked & "/" & appName & ".app") & " && echo yes || echo no") is not "yes" then error "The download does not contain " & appName & ".app, so it was not installed."

		-- Applications if this account may write there (admins can), else the user's own Applications.
		set home to POSIX path of (path to home folder)
		set dest to "/Applications"
		try
			do shell script "test -w /Applications"
		on error
			set dest to home & "Applications"
			do shell script "mkdir -p " & quoted form of dest
		end try
		set appPath to dest & "/" & appName & ".app"

		-- Asset Hub 1.x goes to the Trash, from either Applications folder. Its data folder is never touched.
		my trashLegacyApp(helpers, "/Applications/" & appName & ".app")
		my trashLegacyApp(helpers, home & "Applications/" & appName & ".app")

		-- An existing 2.x copy is replaced in place.
		my quitApp(appPath)
		do shell script "rm -rf " & quoted form of appPath & " && mv " & quoted form of (unpacked & "/" & appName & ".app") & " " & quoted form of appPath
		do shell script "xattr -dr com.apple.quarantine " & quoted form of appPath & " 2>/dev/null; true"
		do shell script "rm -rf " & quoted form of work
		set progress completed steps to 100

		do shell script "open " & quoted form of appPath
		display dialog "Asset Hub " & appVersion & " is installed in " & dest & ". Asset Hub 2 starts fresh: sign in again (the GitHub CLI option is recommended) and recreate your collections. From now on, open it like any other app (you can delete this installer)." buttons {"Done"} default button "Done" with title "Install Asset Hub" with icon note giving up after 20
	on error message number code
		if work is not "" then do shell script "rm -rf " & quoted form of work
		if code is not -128 then display dialog message buttons {"Close"} default button "Close" with title "Install Asset Hub" with icon stop
	end try
end run
