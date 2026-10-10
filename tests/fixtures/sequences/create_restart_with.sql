-- `RESTART WITH n` is a CREATE SEQUENCE option too (`SeqOptElem`), and `init_params`
-- applies it: the first nextval is n, not the start.
-- @setup
CREATE SEQUENCE crst RESTART WITH 5;
-- @test
SELECT nextval('crst');
