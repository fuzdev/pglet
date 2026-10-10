-- `ORDER BY` an array-valued key: the sort compares two `Value::Array`s element-wise and
-- has to carry the member order into each element, which is the half of the rule the
-- comparison sites do not exercise.
--
-- The projected label is `m::text AS label` and the sort key is the `ARRAY[m]`
-- expression, so no output column named `m` can capture the `ORDER BY` (see the harness
-- note in `docs/enums.md`).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id, m::text AS label FROM t ORDER BY ARRAY[m], id;
