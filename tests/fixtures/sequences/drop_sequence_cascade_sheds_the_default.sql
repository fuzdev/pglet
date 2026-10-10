-- `CASCADE` past the `pg_attrdef` edge drops the **default**, never the column: `id`
-- keeps its type, its NOT NULL and its rows, and only `column_default` goes. The
-- explicit-value insert after the CASCADE is part of the assertion — it would fail if
-- the column had gone with the sequence.
-- @setup
CREATE TABLE dsc_t (id SERIAL, b INT);
INSERT INTO dsc_t (b) VALUES (1), (2);
DROP SEQUENCE dsc_t_id_seq CASCADE;
INSERT INTO dsc_t (id, b) VALUES (9, 3);
-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default,
       is_nullable::text AS is_nullable
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'dsc_t'
ORDER BY ordinal_position;
