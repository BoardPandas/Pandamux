@echo off
REM pandamux CMD Integration
REM Reports CWD via OSC 9 escape sequence embedded in prompt

REM Set PANDAMUX env var
set PANDAMUX=1

REM Set prompt to include OSC 9 with current directory
REM ESC]9;9;PATH ESC\ then normal prompt
prompt $e]9;9;$P$e\$P$G
