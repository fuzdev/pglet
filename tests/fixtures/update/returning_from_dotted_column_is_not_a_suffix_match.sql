-- A bare `RETURNING` name whose only claimant is the source, over a target carrying a
-- quoted column whose name **ends with** that name.
--
-- This is the fixture that separates the two ways of closing the dotted-name collision.
-- Resolution by `(relation, column)` pair counts one claimant for `c` — the source's
-- column — and answers `SOURCEC`, which is what 18.6 does. Every *string* rule over the
-- synthesized names gets it wrong instead, in both of its plausible spellings, because
-- `rdn_t.b.c` and `s.c` **both** end in `.c`:
--
-- - exact-then-suffix (what shipped) finds no exact `c` and then those two;
-- - suffix-only — the natural repair, since dropping the exact tier fixes both
--   directions of the collision itself — finds the same two.
--
-- Measured: this file is red under the shipped defect *and* under the suffix-only
-- repair, which is what the two headline fixtures cannot do — both of those go **green**
-- under suffix-only, so on their own they would have waved it through.
-- @setup
CREATE TABLE rdn_t (id INT, "b.c" TEXT);
CREATE TABLE rdn_s (id INT, c TEXT);
INSERT INTO rdn_t VALUES (1, 'TARGETDOT');
INSERT INTO rdn_s VALUES (1, 'SOURCEC');

-- @test
UPDATE rdn_t SET id = rdn_t.id FROM rdn_s s WHERE s.id = rdn_t.id
  RETURNING c AS bare_c, "b.c" AS target_dotted;
