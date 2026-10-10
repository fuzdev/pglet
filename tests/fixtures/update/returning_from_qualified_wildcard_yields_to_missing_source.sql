-- The **control** for `returning_from_wildcard_yields_to_missing_source.sql`: the same
-- statement, spelled `RETURNING <target>.*` instead of `RETURNING *`.
--
-- It answered `42P01` before a plan-time refusal stood over the bare spelling, while
-- that refusal answered `0A000`, and after the refusal was replaced by expanding the
-- wildcard over the whole namespace. The pair is what makes its sibling a *precedence*
-- guard: one statement, two wildcard spellings, and only the spelling ever moved the
-- SQLSTATE — so the divergence was where the bare `*` was handled, not the missing
-- relation.
--
-- It also guards the qualified expansion directly. A `<q>.*` whose qualifier names no
-- relation in the namespace is `42P01` at plan time; here the qualifier *is* the target,
-- so it must expand rather than refuse, and the source's own `42P01` must be what
-- surfaces.
-- @setup
CREATE TABLE wpq_t (id INT, v TEXT);
INSERT INTO wpq_t VALUES (1, 'TARGET');

-- @test
-- @expect-error: 42P01 — the missing FROM source, reported for the qualified wildcard too
UPDATE wpq_t SET id = 1 FROM wpq_nosuch s RETURNING wpq_t.*;
