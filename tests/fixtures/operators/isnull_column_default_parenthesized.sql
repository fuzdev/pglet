-- The control: a parenthesized `DEFAULT` is a full `a_expr`, so the postfix test reads.
-- @setup
CREATE TABLE indp (id INT, a BOOL DEFAULT (NULL ISNULL));
INSERT INTO indp (id) VALUES (1);
-- @test
SELECT id, a FROM indp;
