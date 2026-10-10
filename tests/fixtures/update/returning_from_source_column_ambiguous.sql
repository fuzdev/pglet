-- `UPDATE t SET … FROM s WHERE … RETURNING v`, where **both** relations carry `v`.
--
-- Postgres decides a bare reference's binding over the statement's whole namespace, so
-- two claimants is `42702 column reference "v" is ambiguous`. Both engines reject, so
-- the SQLSTATE is compared here — which is what makes this fixture stronger than one
-- whose oracle accepts.
--
-- Two different builds have answered `TARGET` instead. A namespace that held only the
-- target had exactly one claimant by construction; a namespace that widened to the
-- source but resolved a bare name by first-match would have one again. So the assertion
-- is the SQLSTATE and not merely "some error", and the values differ (`TARGET` against
-- `SOURCE`) so a build that answers says *which* relation it bound.
--
-- Its accepting polarity is `returning_from_source_bare_column.sql` (a bare name only
-- the source carries) and `returning_from_unambiguous_names_answer.sql` (one only the
-- target carries): a fix that refused every bare name over a `FROM` reddens both and
-- leaves this file green.
-- @setup
CREATE TABLE rba_t (id INT, v TEXT);
CREATE TABLE rba_s (id INT, v TEXT, k INT);
INSERT INTO rba_t VALUES (1, 'TARGET');
INSERT INTO rba_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42702 — a bare RETURNING name two relations of the statement claim is ambiguous
UPDATE rba_t SET id = 1 FROM rba_s s WHERE s.id = rba_t.id RETURNING v;
