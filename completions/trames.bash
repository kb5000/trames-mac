# bash completion for trames and trames-dev
_trames() {
    local current="${COMP_WORDS[COMP_CWORD]}" previous="${COMP_WORDS[COMP_CWORD-1]}" command="${COMP_WORDS[0]}"
    case "$previous" in
        -e|--environment) COMPREPLY=( $(compgen -W "$("$command" env ls -q 2>/dev/null)" -- "$current") ) ;;
        env) COMPREPLY=( $(compgen -W 'ls create start stop restart enable disable setup status terminal packages configure import-store backup restore check delete doctor' -- "$current") ) ;;
        config) COMPREPLY=( $(compgen -W 'get set unset list edit path --global' -- "$current") ) ;;
        session) COMPREPLY=( $(compgen -W 'ls attach' -- "$current") ) ;;
        apk) COMPREPLY=( $(compgen -W 'install launch list' -- "$current") ) ;;
        completion) COMPREPLY=( $(compgen -W 'bash zsh fish' -- "$current") ) ;;
        *) COMPREPLY=( $(compgen -W 'shell exec env config session apk gui completion --environment --help --version' -- "$current") ) ;;
    esac
}
complete -F _trames trames trames-dev
