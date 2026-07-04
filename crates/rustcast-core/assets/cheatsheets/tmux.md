# tmux — terminal multiplexer

Prefix defaults to Ctrl-b (shown as C-b).

## Sessions
tmux                            # start
tmux new -s work                # named session
tmux ls                         # list sessions
tmux attach -t work             # attach
tmux kill-session -t work
C-b d                           # detach
C-b $                           # rename session

## Windows
C-b c                           # create window
C-b ,                           # rename window
C-b n / p                       # next / previous
C-b 0..9                        # jump to window N
C-b w                           # window list
C-b &                           # kill window

## Panes
C-b %                           # split vertical
C-b "                           # split horizontal
C-b arrow                       # move between panes
C-b o                           # cycle panes
C-b x                           # kill pane
C-b z                           # zoom/unzoom pane
C-b space                       # cycle layouts
C-b { / }                       # swap pane

## Copy mode
C-b [                           # enter copy mode (q to exit)
C-b ]                           # paste
