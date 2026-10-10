-- `"EXCLUDED"` is a **quoted** identifier, so it keeps its case and names no relation:
-- the pseudo-relation is `excluded`, and 18.6 answers `42P01 "EXCLUDED"`. pglet wrote the
-- excluded value, from the same qualifier-blind bare loop as
-- `insert/on_conflict_do_update_bogus_qualifier_loud.sql` — the quoting is the whole
-- difference, which is what makes this the case-fold rule's own reject polarity.
--
-- Its accept polarities are `EXCLUDED.v` and `"excluded".v`, both of which answer on both
-- engines (the unquoted spelling folds, the quoted-lowercase one already matches).
-- @setup
CREATE TABLE ocq_t (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocq_t VALUES (1, 'orig');

-- @expect-error: missing FROM-clause entry for table "EXCLUDED" (42P01)
-- @no-profile-lock: the arbiter's unique-index probe is constraint enforcement rather than acceleration and runs identically on both profiles; the refusal is raised evaluating the DO UPDATE action, which selects no index
-- @test
INSERT INTO ocq_t VALUES (1, 'new') ON CONFLICT (id) DO UPDATE SET v = "EXCLUDED".v;
