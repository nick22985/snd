# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_snd_global_optspecs
	string join \n h/help
end

function __fish_snd_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_snd_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_snd_using_subcommand
	set -l cmd (__fish_snd_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c snd -n "__fish_snd_needs_command" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_needs_command" -a "add" -d 'Add a new server'
complete -c snd -n "__fish_snd_needs_command" -a "remove" -d 'Remove a server'
complete -c snd -n "__fish_snd_needs_command" -a "edit" -d 'Edit a server\'s target'
complete -c snd -n "__fish_snd_needs_command" -a "list" -d 'List all configured servers'
complete -c snd -n "__fish_snd_needs_command" -a "completions" -d 'Generate shell completions'
complete -c snd -n "__fish_snd_needs_command" -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c snd -n "__fish_snd_using_subcommand add" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_using_subcommand remove" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_using_subcommand edit" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_using_subcommand list" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_using_subcommand completions" -s h -l help -d 'Print help'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "add" -d 'Add a new server'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "remove" -d 'Remove a server'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "edit" -d 'Edit a server\'s target'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "list" -d 'List all configured servers'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "completions" -d 'Generate shell completions'
complete -c snd -n "__fish_snd_using_subcommand help; and not __fish_seen_subcommand_from add remove edit list completions help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
