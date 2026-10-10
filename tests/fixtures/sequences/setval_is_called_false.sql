-- setval(seq, n, false) sets is_called = false, so the next nextval returns
-- exactly n (not n + 1).
-- @setup
CREATE SEQUENCE seq_setval3;
SELECT setval('seq_setval3', 10, false);
-- @test
SELECT nextval('seq_setval3');
