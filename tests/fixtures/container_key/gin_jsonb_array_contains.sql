-- A GIN index over `jsonb[]` keys each element by `jsonb`'s equality: a probe element
-- `{"a": 1.0}` finds the rows holding `{"a": 1}`, and a row whose array is empty or NULL
-- holds no entry.
-- @profile-lock
-- @setup
CREATE TABLE ckg_ja (id INT, arr JSONB[]);
CREATE INDEX ckg_ja_ix ON ckg_ja USING gin (arr);
INSERT INTO ckg_ja (arr, id) VALUES (ARRAY['1'::jsonb, '"x"'::jsonb], 1), (ARRAY['{"a":1}'::jsonb], 2), ('{}', 3), (NULL, 4), (ARRAY['{"a":1}'::jsonb, '2'::jsonb], 5), (ARRAY['[]'::jsonb, 'null'::jsonb], 6);
-- @test
SELECT id FROM ckg_ja WHERE arr @> ARRAY['{"a":1.0}'::jsonb] ORDER BY id;
