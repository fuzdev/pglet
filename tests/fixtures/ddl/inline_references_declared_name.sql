-- A column-level `CONSTRAINT <n> REFERENCES …` keeps its declared name. The parser
-- accepted the `CONSTRAINT` prefix and then threw the name away, synthesizing
-- `<table>_<col>_fkey` instead — a plausible-but-wrong name an introspecting client
-- cannot detect, and the same silent-wrong class as reporting no name at all.
-- @setup
CREATE TABLE irdn_p (id INT PRIMARY KEY);
CREATE TABLE irdn (a INT CONSTRAINT irdn_myfk REFERENCES irdn_p(id));

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'irdn' AND constraint_type = 'FOREIGN KEY'
ORDER BY constraint_name;
