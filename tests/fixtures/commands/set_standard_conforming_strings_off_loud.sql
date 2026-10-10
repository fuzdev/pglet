-- pglet-stricter, and loud: turning `standard_conforming_strings` off.
-- 18.6 takes the statement and acts on it (a backslash in an ordinary string literal becomes an escape); pglet has one string-literal reading, the standard one, and refuses the statement
-- rather than accept it and go on answering as though it had not been
-- sent. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE (`0A000`) and the
-- message are pinned in `catalog::commands`'s unit tests.
-- @test
-- @expect-error: pglet's standard_conforming_strings is fixed at on
SET standard_conforming_strings = off;
