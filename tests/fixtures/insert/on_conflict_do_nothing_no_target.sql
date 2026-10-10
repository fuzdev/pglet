-- A target-less ON CONFLICT DO NOTHING arbitrates against every unique/PK
-- constraint: conflicting rows (VALUES and INSERT…SELECT forms) are skipped, the
-- original row is preserved, and a non-conflicting row still inserts.
-- @profile-lock
-- @setup
CREATE TABLE oc_t (id INT PRIMARY KEY, v TEXT);
INSERT INTO oc_t (id, v) VALUES (1, 'a');
INSERT INTO oc_t (id, v) VALUES (1, 'b') ON CONFLICT DO NOTHING;
INSERT INTO oc_t (id, v) SELECT 1, 'c' ON CONFLICT DO NOTHING;
INSERT INTO oc_t (id, v) VALUES (2, 'z') ON CONFLICT DO NOTHING;

-- @test
SELECT id, v FROM oc_t ORDER BY id;
