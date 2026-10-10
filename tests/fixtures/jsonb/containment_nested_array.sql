-- An array nested inside a matched object key takes the array rule, recursively:
-- `{"tags":[1,2,3]} @> {"tags":[1]}`. This is the shape a real document store hits
-- constantly (a tag/label list on a row), and pglet returned **false** for it —
-- the object rule matched the key, then compared the two arrays with `==`.
--
-- Nesting must still line up. Postgres's own comment: the mapping of rhs container
-- nodes onto lhs nodes is injective and parent-child edges must map to parent-child
-- edges, so a value found at the wrong depth is not containment.
-- @setup
CREATE TABLE jcna (id INT, d JSONB);
INSERT INTO jcna VALUES
  (1, '{"tags":[1,2,3],"n":1}'),
  (2, '{"tags":[9],"n":2}'),
  (3, '{"a":{"b":[{"c":1}]}}');
-- @test
SELECT
  ('{"tags":[1,2,3]}'::jsonb @> '{"tags":[1]}'::jsonb) AS nested_array,
  ('{"tags":[1,2,3]}'::jsonb @> '{"tags":[1,9]}'::jsonb) AS missing_element,
  ('{"a":{"b":[{"c":1}]}}'::jsonb @> '{"a":{"b":[{"c":1}]}}'::jsonb) AS deep_nested,
  ('{"a":{"b":1,"c":2}}'::jsonb @> '{"a":{"b":1}}'::jsonb) AS nested_object,
  ('[[1,2]]'::jsonb @> '[[1]]'::jsonb) AS array_in_array,
  (SELECT count(*) FROM jcna WHERE d @> '{"tags":[1]}'::jsonb) AS from_column;
