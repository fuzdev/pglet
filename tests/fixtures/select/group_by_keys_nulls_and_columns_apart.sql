-- @test
-- `GROUP BY` over two text keys: two NULLs are one group, a NULL is not an empty string,
-- and the key columns stay apart, so `('ab','c')` and `('a','bc')` are two groups.
SELECT a, b, count(*)
FROM (VALUES ('1', NULL::text), ('1', NULL), ('1', ''), (NULL, ''), (NULL, NULL),
             ('ab', 'c'), ('a', 'bc'), ('ab', 'c')) v(a, b)
GROUP BY a, b
ORDER BY a, b;
