-- A fold whose typed branches are `jsonb` is a `jsonb` side — `coalesce`, `CASE`,
-- `greatest`, nested — its untyped branches taking `jsonb` too, so a row that takes the
-- literal branch merges as well; one whose branches were all untyped stays `text`, as on
-- 18.6.
-- @setup
CREATE TABLE jfs (id INT, j JSONB);
INSERT INTO jfs VALUES (1, '{"k": [1]}'), (2, '{"x": 1}');
-- @test
SELECT id, coalesce(j, '{}') || '{"y": 1}' AS coalesced, CASE WHEN true THEN j END || '{"y": 1}' AS cased,
       greatest(j, j) || '{"y": 1}' AS greatest_of, coalesce(j -> 'k', '[]') || '[2]' AS arrow_fold,
       coalesce(to_jsonb(s), '{}') || '[2]' AS to_jsonb_fold,
       coalesce(coalesce(j, '{}'), '{}') || '{"z": 1}' AS nested,
       coalesce(CASE WHEN id = 1 THEN j END, '{}') || '{"z": 1}' AS case_fold,
       coalesce(j || '{"a": 1}', '{}') || '{"b": 1}' AS concat_fold,
       j || coalesce(NULL, '{"q": 1}') AS text_fold
FROM jfs, (SELECT 1 AS a) s ORDER BY id;
