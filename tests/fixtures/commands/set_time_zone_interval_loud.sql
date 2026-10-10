-- pglet-stricter, and loud: the `INTERVAL` spelling of an offset.
-- 18.6 takes the statement and acts on it (`SHOW TimeZone` answers the new zone, and a `timestamptz` renders and casts in it); pglet has one session zone, UTC, and refuses the statement
-- rather than accept it and go on answering as though it had not been
-- sent. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE is `42601` here, the
-- one-value reader not reading this spelling, pinned in `catalog::commands`'s unit tests.
-- @test
-- @expect-error: pglet's TimeZone is fixed at UTC
SET TIME ZONE INTERVAL '-08:00' HOUR TO MINUTE;
