-- The same unified column in a **predicate** rather than an `ORDER BY`: this answers a
-- different *row*, not a different order, which is the half a sort-only fixture cannot
-- see. Under the label bytes `x > 'ok'` selects `sad`; under the member order it selects
-- `happy`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, x::text AS lbl
FROM (SELECT id, m AS x FROM t UNION ALL SELECT 0, 'ok') q
WHERE x > 'ok'
ORDER BY id;
