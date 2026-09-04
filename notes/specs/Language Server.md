# Language Server

What a name in the source means is a question for a language server, run over the open
project's directory. It is rust-analyzer unless the project names another program.

Nothing starts one by itself. A control at the right of the top bar, left of the chevrons,
starts and stops it. It is marked `LSP` and drawn as its state: text alone when it is off, a
border under the pointer and while a server is there, a colour of its own while one runs, a
turning icon while one is starting or reading the project, and a failure's colour when one
would not start. Hovering it says the same in words. It is off when the app opens, however
it was left. A project with no directory has nothing to run a server over, and the control
is dim and does nothing.

One server runs at a time. Switching projects stops it, and closing the window stops it and
everything it started.

## Agreeing to it

A language server runs the project's own build scripts and macros, so the first start over a
directory is asked about, whichever button asked for it. A question appears under the top
bar naming the program, what running it means, and the directory, with a button for each
answer. Agreeing starts it and is saved with the project, so that directory is not asked
about again, now or in a later run. Declining starts nothing and is saved nowhere.

What was agreed to is the directory, so editing the project's directory takes the agreement
back. Another project brings its own answer, and is not asked again for having been away.

## The project's own settings

A project can keep settings for the server in `.vscode/settings.json` under its directory.
Some trees cannot be read without them. Most projects have no such file, which is not an
error and is not remarked on.

The keys that begin `rust-analyzer.` are sent to the server, with that prefix dropped and
the rest of the name split on its dots, over what the app asks of every server. The other
keys are the editor's own and are skipped without a word. Comments and a trailing comma are
allowed, as they are in an editor. `${workspaceFolder}` in a value is the project's
directory; every other `${...}` is an error, as are a file that is not an object of JSON and
a name given both a value and a table. An error stops the server from starting.

## In the Project view

A section of the view holds the program to run, empty for the usual one and saved with the
project; a button that starts and stops the server; whether this directory has been agreed
to, with a button to take that back, which also stops the server it was given for; and what
the project's settings gave the server, one line per setting, running or not. A line says how
it is going: running, still reading the project, or why it would not start. A project with no
directory says that instead.

## When there is no answer

A language server that is not installed, or will not start, stops nothing else, and the
control and that section are the only places it is said. One that runs and then ends at once
is not a server that stopped answering: what it said on its way out is shown.

A question asked while a server is still starting is answered once it is ready, and a later
question of the same kind drops one still waiting. With no server running, whatever would
have asked does nothing.
