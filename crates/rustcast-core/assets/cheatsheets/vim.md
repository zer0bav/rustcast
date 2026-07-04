# vim — quick reference

## Modes
i a o        insert (before / after / new line)
Esc          normal mode
v V C-v      visual / line / block
:            command mode

## Movement
h j k l      left down up right
w b e        word forward / back / end
0 ^ $        line start / first non-blank / end
gg G         file top / bottom
:N           go to line N
{ }          paragraph up / down
Ctrl-d/u     half page down / up

## Editing
x            delete char
dd yy p      delete line / yank line / paste
dw cw        delete / change word
u  Ctrl-r    undo / redo
r  R         replace char / replace mode
. (dot)      repeat last change
>> <<        indent / unindent

## Search & replace
/text  n N              search, next, prev
:%s/old/new/g           replace all in file
:%s/old/new/gc          replace with confirm

## Files
:w  :q  :wq  :q!        write / quit / both / force quit
:e file                 open file
:sp  :vsp               split / vsplit
