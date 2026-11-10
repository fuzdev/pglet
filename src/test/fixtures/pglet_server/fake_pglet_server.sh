#!/bin/sh
# A stand-in for pglet_server, for src/test/pglet_server.test.ts. FAKE_MODE picks the
# behavior, FAKE_DIR is where it records its pid and arguments.
echo "$$" > "$FAKE_DIR/pid"
echo "$*" > "$FAKE_DIR/args"
case "$FAKE_MODE" in
ready)
	echo 'pglet using B+tree storage (in-memory, no persistence)'
	echo 'pglet listening on 127.0.0.1:45678'
	exec sleep 300
	;;
split)
	printf 'pglet using B+tree storage (in-memory, no persistence)\npglet listen'
	sleep 0.2
	printf 'ing on 127.0.0.1:45679\n'
	exec sleep 300
	;;
silent)
	exec sleep 300
	;;
exit)
	echo 'pglet_server: --port: invalid port "x"' >&2
	exit 3
	;;
wildcard)
	echo 'pglet listening on 0.0.0.0:45678'
	exec sleep 300
	;;
noisy)
	echo 'pglet listening on 127.0.0.1:45680'
	head -c 4194304 /dev/zero | tr '\0' 'e' >&2
	head -c 4194304 /dev/zero | tr '\0' 'o'
	: > "$FAKE_DIR/drained"
	exec sleep 300
	;;
stubborn)
	trap '' TERM
	echo 'pglet listening on 127.0.0.1:45681'
	while :; do sleep 1; done
	;;
esac
