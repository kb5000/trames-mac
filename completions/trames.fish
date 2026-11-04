for command in trames trames-dev
    complete -c $command -f
    complete -c $command -s e -l environment -r -a "($command env ls -q 2>/dev/null)"
    complete -c $command -n 'not __fish_seen_subcommand_from shell exec env config session gui completion' -a 'shell exec env config session gui completion'
    complete -c $command -n '__fish_seen_subcommand_from env' -a 'ls create start stop restart enable disable setup status terminal packages configure import-store backup restore check delete doctor'
    complete -c $command -n '__fish_seen_subcommand_from config' -a 'get set unset list edit path --global'
    complete -c $command -n '__fish_seen_subcommand_from session' -a 'ls attach'
    complete -c $command -n '__fish_seen_subcommand_from completion' -a 'bash zsh fish'
    complete -c $command -s h -l help
    complete -c $command -s V -l version
end
