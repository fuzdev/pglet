-- 42703 sends no `t`/`c` on any of its four routes, though pglet's own variant carries
-- both names. It also sends a `P`, and (with a near name) an `H`, which pglet cannot
-- derive — both omitted rather than guessed.
-- @setup
CREATE TABLE t_cnf (id INT, name TEXT);

-- @test
-- @expect-error: no such column
INSERT INTO t_cnf (idd) VALUES (1);
