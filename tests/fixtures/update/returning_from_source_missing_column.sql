-- A `RETURNING` reference qualified by a relation that **is** in the namespace, naming a
-- column it does not carry: `42703` on both engines.
--
-- The other half of the pair `returning_from_source_name_hidden_by_alias.sql` opens. The
-- two spellings differ only in whether the qualifier resolves, and Postgres gives them
-- different SQLSTATEs for that reason (`errorMissingRTE` against `errorMissingColumn`).
-- A namespace that answered `42P01` for both — which is what a scope holding only the
-- target does — passes neither file's oracle.
-- @setup
CREATE TABLE rmc_t (id INT, v TEXT);
CREATE TABLE rmc_s (id INT, sv TEXT, k INT);
INSERT INTO rmc_t VALUES (1, 'TARGET');
INSERT INTO rmc_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42703 — the qualifier resolves, the column does not
UPDATE rmc_t SET v = 'hit' FROM rmc_s s WHERE s.id = rmc_t.id RETURNING s.nosuch;
