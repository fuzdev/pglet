-- pglet-stricter, and loud: the `set_config` spelling.
-- 18.6 takes the statement and acts on it (`SHOW IntervalStyle` answers the new style, and an `interval` renders in it); pglet has one interval style, `postgres`, and refuses the statement
-- rather than accept it and go on answering as though it had not been
-- sent. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE (`0A000`) and the
-- message are pinned in `catalog::commands`'s unit tests.
-- @test
-- @expect-error: pglet's IntervalStyle is fixed at postgres
SELECT set_config('IntervalStyle', 'postgres_verbose', false);
