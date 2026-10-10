-- The second shape of the same defect: a bound `$N` compared with a source column the
-- **target does not carry at all**, so the old catalog-keyed walk had no fallback to
-- reach and reported no type — OID `0`, which is what the parameter-inference module's
-- own doc calls the value a binary-protocol client's introspection trips over. 18.6
-- reports `int4`.
--
-- Its twin (`returning_from_source_param_type.sql`) is the *wrong-type* shape; this one
-- is the *no-type* shape, and they can regress independently, which is why both exist.
-- @setup
CREATE TABLE rpo_t (id INT, v TEXT);
CREATE TABLE rpo_s (id INT, k INT);
INSERT INTO rpo_t VALUES (1, 'a');
INSERT INTO rpo_s VALUES (1, 7);

-- @test
-- @param $1 int4 7
UPDATE rpo_t SET id = rpo_t.id FROM rpo_s s WHERE s.id = rpo_t.id RETURNING (s.k = $1) AS eq;
