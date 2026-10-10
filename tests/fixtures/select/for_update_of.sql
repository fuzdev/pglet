-- @profile-lock
-- @setup
-- A real row-level locking clause with modifiers parses (and, inside a block, takes
-- pglet's writer): `FOR NO KEY UPDATE OF <table>`. The result matches Postgres — a lock
-- nobody contends for can't change a query's outcome here. The
-- `NOWAIT` this fixture used to carry is now refused (`select/for_update_nowait_loud`):
-- a wait policy is the one part of the clause whose absence a second connection sees.
CREATE TABLE lockt (id INT PRIMARY KEY, code TEXT);
INSERT INTO lockt VALUES (1, 'a'), (2, 'b'), (3, 'c');

-- @test
SELECT id, code FROM lockt WHERE id > 1 ORDER BY id FOR NO KEY UPDATE OF lockt;
