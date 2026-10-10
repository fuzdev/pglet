-- jsonb ORDER BY follows Postgres's B-tree order (`compareJsonbContainers`): the
-- type rank `null < string < number < bool < array < object`, then content — NOT raw
-- JSON text order. Regression guard for the `to_json_string().cmp()` arm (which put
-- e.g. `false`/`true` and numbers in text-lexicographic, not type-rank, order).
-- @setup
CREATE TABLE j (id INT, v JSONB);
INSERT INTO j VALUES
  (1, 'null'), (2, '"a"'), (3, '"b"'), (4, '1'), (5, '2'),
  (6, 'false'), (7, 'true'), (8, '[1]'), (9, '[1,2]'),
  (10, '{"a":1}'), (11, '{"a":1,"b":2}');
-- @test
SELECT id, v FROM j ORDER BY v, id;
