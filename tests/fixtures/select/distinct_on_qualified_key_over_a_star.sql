-- @setup
CREATE TABLE dqs (a INT, b INT, t TEXT);
INSERT INTO dqs VALUES (1, 5, 'x'), (1, 7, 'y'), (2, 5, 'z'), (3, 9, 'w');

-- An unexpanded bare `*` projects every column of its single FROM relation under that
-- column's own name, so a **qualified** key denotes one of them and needs no junk
-- column. The denotation walk compares a key against projection *entries*, and a bare
-- `*` is an `Expr::Column { name: "*" }` matching none — so this ordinary statement was
-- `0A000` while its bare-key spelling (`DISTINCT ON (a)`) answered.
-- @test
SELECT DISTINCT ON (dqs.a) * FROM dqs ORDER BY dqs.a, b DESC;
