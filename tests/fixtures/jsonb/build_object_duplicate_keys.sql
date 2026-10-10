-- `jsonb_build_object` returns **jsonb**, so last-wins key dedup applies to it —
-- it is an invariant of the type, not a quirk of the text parser.
--
-- pglet built `{"a": 1, "a": 2}`: a shape no `::jsonb` cast can produce, diverging
-- from Postgres's `{"a": 2}` *and* internally inconsistent, since the readers that
-- then walked it (`->`, `?`, `@>`) take the **first** match while the text path
-- dedups to the **last**. So `jsonb_build_object('a',1,'a',2) -> 'a'` answered `1`
-- where the identical literal answered `2`.
-- @setup
CREATE TABLE jbod (id INT);
INSERT INTO jbod VALUES (1);
-- @test
SELECT
  jsonb_build_object('a', 1, 'a', 2) AS deduped,
  jsonb_build_object('a', 1, 'a', 2) -> 'a' AS reader_agrees,
  jsonb_build_object('k', jsonb_build_object('a', 1, 'a', 2)) AS nested_deduped,
  (jsonb_build_object('a', 1, 'a', 2) = '{"a":2}'::jsonb) AS equals_literal,
  jsonb_build_object('b', 1, 'aa', 2, 'c', 3) AS canonical_key_order
FROM jbod;
