-- The same unknown-literal resolution inside `HAVING`. `HAVING x = '1'` over an int
-- column is an integer comparison in Postgres; before the clause-coercion pass
-- covered `HAVING`, the literal reached the operand gate as text and raised a
-- spurious `42883`.
-- @setup
CREATE TABLE gt (x INT, tag TEXT);
INSERT INTO gt VALUES (1, 'a'), (1, 'b'), (2, 'c');
-- @test
SELECT x AS k, count(*) AS n FROM gt GROUP BY x HAVING x = '1' ORDER BY k;
