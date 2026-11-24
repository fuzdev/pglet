-- `ALTER COLUMN … DROP NOT NULL` lets an existing column hold NULL, and drops the column's
-- `NOT NULL` constraint with the flag. A column already nullable is a success that changes
-- nothing; the `COLUMN` noise word is optional.
-- @profile-lock
-- @setup
CREATE TABLE dnn_t (id int PRIMARY KEY, a int NOT NULL, b text NOT NULL, c text);
INSERT INTO dnn_t VALUES (1, 9, 'x', NULL);
ALTER TABLE dnn_t ALTER COLUMN a DROP NOT NULL;
ALTER TABLE dnn_t ALTER b DROP NOT NULL;
ALTER TABLE dnn_t ALTER COLUMN c DROP NOT NULL;
INSERT INTO dnn_t VALUES (2, NULL, NULL, NULL);
-- @test
SELECT t.id, t.a, t.b,
       (SELECT n.n FROM (SELECT count(*) AS n FROM pg_constraint c JOIN pg_class k ON k.oid = c.conrelid
         WHERE k.relname::text = 'dnn_t' AND c.contype::text = 'n') AS n) AS not_null_constraints,
       (SELECT a.attnotnull FROM pg_attribute a JOIN pg_class k ON k.oid = a.attrelid
         WHERE k.relname::text = 'dnn_t' AND a.attname::text = 'a') AS a_not_null
FROM dnn_t t ORDER BY t.id;
