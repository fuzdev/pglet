-- JSONB `@>` over arrays is **subset with membership**, not equality: every element
-- of the right operand must appear somewhere in the left, order irrelevant and
-- multiplicity ignored (`'[1]' @> '[1,1]'` is true — Postgres matches each rhs
-- element independently).
--
-- pglet implemented only object-vs-object containment and fell back to a structural
-- `a == b` for every other pair, so all of these returned **false**: a plausible
-- wrong boolean rather than a loud failure, on the half of the operator that does
-- the work.
-- @setup
CREATE TABLE jcas (id INT, d JSONB);
INSERT INTO jcas VALUES (1, '[1,2,3]'), (2, '["a","b"]'), (3, '[]');
-- @test
SELECT
  ('[1,2,3]'::jsonb @> '[1,3]'::jsonb) AS subset,
  ('[1,2,3]'::jsonb @> '[3,1]'::jsonb) AS order_irrelevant,
  ('[1]'::jsonb @> '[1,1]'::jsonb) AS multiplicity_ignored,
  ('[1,2]'::jsonb @> '[1,2,3]'::jsonb) AS not_a_superset,
  (SELECT count(*) FROM jcas WHERE d @> '[1]'::jsonb) AS from_column;
