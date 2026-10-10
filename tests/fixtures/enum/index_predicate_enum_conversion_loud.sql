-- A partial predicate converting a `text` column to an enum: 18.6 refuses it, `enum_in`
-- not being `IMMUTABLE` — ahead of a missing key column, which `DefineIndex` resolves
-- after the predicate. Accepted, it dumped to a `CREATE INDEX` stock Postgres refuses.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ip (id INT, s TEXT);

-- @profile-lock
-- @test
-- @expect-error: enum_in is not immutable
-- @expect-message
CREATE INDEX ip_ix ON ip (nosuchcol) WHERE s::mood = 'ok';
