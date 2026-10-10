-- The **accept** polarity beside `update/returning_from_source_column_ambiguous.sql`,
-- and the fixture an over-correction reddens.
--
-- Making a bare name two relations claim `42702` must not make *every* bare name an
-- error, and expanding a wildcard must not lose its qualifier. Both spellings here are
-- ordinary SQL that 18.6 and pglet agree on, and both are one edit away from a refusal:
--
-- * `o.*` — the qualified target wildcard, which is what the spine's own
--   `UPDATE role_grant_offer o … FROM actor a … RETURNING o.*` writes. It expands to
--   **qualified** references (`ExpandSingleTable`), and the qualifier is load-bearing:
--   expanded unqualified, `o.*`'s `v` would be re-checked as a bare name, find the
--   source's `v` among the claimants, and answer `42702` — turning the spine's query
--   into an error.
-- * `only_target` — a bare name the source does **not** carry, so it has one claimant
--   and binds, exactly as it does on 18.6. A fix that refused every bare name whenever
--   the statement has a `FROM` would redden here and nowhere else.
--
-- The tables differ in arity (3 columns against 2) and in values, so a wildcard that
-- expanded against the wrong relation is visible in the shape *and* in the data. `v` is
-- shared and therefore never referenced bare; the label on the second item keeps the
-- output column names distinct for the harness's JSON keying. Wrapped in a CTE so the
-- body can `ORDER BY` for a deterministic comparison.
-- @setup
CREATE TABLE rbu_t (id INT, v TEXT, only_target INT);
CREATE TABLE rbu_s (id INT, v TEXT);
INSERT INTO rbu_t VALUES (1, 'TARGET', 42), (2, 'TARGET2', 43);
INSERT INTO rbu_s VALUES (1, 'SOURCE'), (2, 'SOURCE2');

-- @test
WITH updated AS (
  UPDATE rbu_t o
  SET only_target = only_target + 100
  FROM rbu_s s
  WHERE s.id = o.id
  RETURNING o.*, only_target AS bare_only_target
)
SELECT * FROM updated ORDER BY id;
