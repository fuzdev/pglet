-- The claims need not be adjacent — `CopyGetAttnums` tests each resolved attribute
-- against every one it has collected. pglet emitted three fields for two columns.
-- @setup
CREATE TABLE cdtn (a int, b int, c int);
INSERT INTO cdtn VALUES (1, 2, 3);
-- @copy-to
COPY cdtn (a, b, a) TO STDOUT;
