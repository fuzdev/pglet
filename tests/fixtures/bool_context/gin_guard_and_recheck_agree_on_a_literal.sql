-- The GIN route decides the same predicate twice: a scan-independent disjunct is
-- lifted into a guard the scan evaluates, and the recheck `Filter` still carries
-- the whole predicate. A row admitted only by the guard matches no GIN probe and
-- never reaches the `Filter`, so widening one alone loses rows on the accelerated
-- profile only — invisible to the stripped one.
-- @setup
CREATE TABLE bcgin (id INT, tags TEXT[]);
CREATE INDEX bcgin_ix ON bcgin USING gin (tags);
INSERT INTO bcgin VALUES (1, '{a}'), (2, '{b}'), (3, '{c}');
-- @profile-lock
-- @test
SELECT id FROM bcgin WHERE tags @> '{a}' OR 't' ORDER BY id;
