-- A row whose array holds one `jsonb` key twice — `1` and `1.0` — holds one entry
-- under it, and an update or a delete of the row takes that entry out: the probes
-- after each write find what a scan finds.
-- @profile-lock
-- @setup
CREATE TABLE ckr_ja (id INT, arr JSONB[]);
CREATE INDEX ckr_ja_ix ON ckr_ja USING gin (arr);
INSERT INTO ckr_ja (arr, id) VALUES (ARRAY['1'::jsonb, '1.0'::jsonb, '1.00'::jsonb], 1), (ARRAY['1'::jsonb, '2'::jsonb], 2), (ARRAY['1.0'::jsonb, '1'::jsonb], 3);
UPDATE ckr_ja SET arr = ARRAY['2'::jsonb, '2.0'::jsonb] WHERE id = 1;
DELETE FROM ckr_ja WHERE id = 3;
INSERT INTO ckr_ja (arr, id) VALUES (ARRAY['1.000'::jsonb], 4);
-- @test
SELECT 'one' AS probe, id FROM ckr_ja WHERE arr @> ARRAY['1'::jsonb]
UNION ALL
SELECT 'two', id FROM ckr_ja WHERE arr @> ARRAY['2.0'::jsonb]
ORDER BY probe, id;
