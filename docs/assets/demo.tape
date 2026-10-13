# Renders docs/assets/demo.gif for the README.
# Requires VHS (`brew install vhs`) and `vigilia` on PATH. From the repo root:
#   vhs docs/assets/demo.tape
# It runs the real binary: the Mac stays awake while it records, and the
# session is stopped at the end.

Output docs/assets/demo.gif

Require vigilia

Set Shell "bash"
Set FontSize 20
Set Width 1000
Set Height 340
Set Padding 28
Set Theme "Catppuccin Mocha"
Set TypingSpeed 55ms
Set WindowBar Colorful

Hide
Type "export PS1='$ '; vigilia stop >/dev/null 2>&1; clear"
Enter
Show

Sleep 600ms
Type "vigilia start 2h"
Sleep 400ms
Enter
Sleep 2s

Type "vigilia extend 30m"
Sleep 400ms
Enter
Sleep 2s

Type "vigilia start 3h --extend   # what agents and scripts run"
Sleep 400ms
Enter
Sleep 2500ms

Type "vigilia stop"
Sleep 400ms
Enter
Sleep 3s
