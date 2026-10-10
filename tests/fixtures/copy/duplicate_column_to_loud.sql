-- A COPY column list is a set, in the **TO** direction as well as in `FROM`: both go
-- through the one `CopyGetAttnums` (`commands/copy.c`), so `COPY t (a, a) TO STDOUT` is
-- `42701 column "a" specified more than once`. pglet emitted the column twice.
-- @setup
CREATE TABLE cdt (a int, b int, c int);
INSERT INTO cdt VALUES (1, 2, 3);
-- @copy-to
COPY cdt (a, a) TO STDOUT;
