-- setval(seq, n) sets the counter so the next nextval returns n + 1
-- (is_called defaults to true).
-- @setup
CREATE SEQUENCE seq_setval2;
SELECT setval('seq_setval2', 10);
-- @test
SELECT nextval('seq_setval2');
