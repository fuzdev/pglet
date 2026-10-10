-- The control that isolates the repeat. With no repeat in the `SET` list both engines
-- answer the row-source cell's own `22P02`, so the clause is not what decides this.
--
-- Add the repeat and the two engines split — 18.6 still answers `22P02` and pglet answers
-- the `SET` list's `42601` — and **that statement cannot be a fixture**: both engines
-- error and the SQLSTATEs differ, which `@expect-error` scores red. `docs/names.md`
-- § What the refusals do not carry is where it is recorded instead.
-- @setup
CREATE TABLE ocdcast (a INT UNIQUE, b INT, bo BOOL);
INSERT INTO ocdcast VALUES (1, 1, true);

-- @expect-error: invalid input syntax for type integer: "notanint" (22P02)
-- @profile-lock
-- @test
INSERT INTO ocdcast VALUES (5, 'notanint', true) ON CONFLICT (a) DO UPDATE SET b = 1;
