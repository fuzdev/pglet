-- A `--` comment ends at a carriage return, as `scan.l`'s does (`non_newline` is
-- `[^\n\r]`): the `WHERE` after a lone carriage return on the comment's line is read,
-- so the `DELETE` removes one row. A comment that ran on to the next line feed would
-- swallow the `WHERE` and empty the table.
-- @setup
CREATE TABLE crt (a INT);
INSERT INTO crt VALUES (1), (2), (3);
DELETE FROM crt -- only oneWHERE a = 1;
-- @test
SELECT a FROM crt ORDER BY a;
