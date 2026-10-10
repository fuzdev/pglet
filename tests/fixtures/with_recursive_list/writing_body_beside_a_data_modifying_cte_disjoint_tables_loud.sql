-- A writing body beside a data-modifying CTE on another table is refused too: the refusal
-- is keyed on the body writing, not on the tables it names.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
CREATE TABLE u (y INT);
-- @expect-error: pglet refuses a writing body beside a data-modifying CTE (18.6 answers)
-- @test
WITH w AS (INSERT INTO u VALUES (9) RETURNING y) INSERT INTO s SELECT 100 FROM w;
