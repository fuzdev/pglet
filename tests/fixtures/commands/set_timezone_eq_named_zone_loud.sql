-- pglet-stricter, and loud: the generic `SET <name> =` spelling.
-- 18.6 takes the statement and acts on it (`SHOW TimeZone` answers the new zone, and a `timestamptz` renders and casts in it); pglet has one session zone, UTC, and refuses the statement
-- rather than accept it and go on answering as though it had not been
-- sent. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE (`0A000`) and the
-- message are pinned in `catalog::commands`'s unit tests.
-- @test
-- @expect-error: pglet's TimeZone is fixed at UTC
SET TimeZone = 'America/New_York';
