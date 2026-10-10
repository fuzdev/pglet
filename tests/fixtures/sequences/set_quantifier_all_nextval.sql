-- `ALL` on the write member: 18.6 accepts it and the sequence advances normally, so the
-- accept side of the quantifier has a state observable in the answer rather than only in
-- a value. pglet refused this 42601 before.
-- @setup
CREATE SEQUENCE sq_na;
-- @test
SELECT nextval(ALL 'sq_na') AS n;
