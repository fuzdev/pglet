-- `FOR READ ONLY` is `gram.y`'s other `for_locking_clause` production — the SQL92
-- spelling, a genuine no-op on 18.6 (it asserts the cursor is not updatable). pglet used
-- to refuse it `42601` for want of the arm.
-- @setup
CREATE TABLE lro (id INT, code TEXT);
INSERT INTO lro VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id, code FROM lro ORDER BY id FOR READ ONLY;
