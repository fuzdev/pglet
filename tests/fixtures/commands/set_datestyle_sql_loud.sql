-- pglet-stricter, and loud: an output format other than ISO, as a bare word.
-- 18.6 takes the statement and acts on it (`SHOW DateStyle` answers the new style, and a `date` renders in it); pglet has one date style, `ISO, MDY`, and refuses the statement
-- rather than accept it and go on answering as though it had not been
-- sent. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE (`0A000`) and the
-- message are pinned in `catalog::commands`'s unit tests.
-- @test
-- @expect-error: pglet's DateStyle is fixed at ISO, MDY
SET datestyle TO sql;
