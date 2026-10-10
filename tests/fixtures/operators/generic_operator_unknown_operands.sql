-- The operand types the refusal names are 18.6's: an unadorned literal is `unknown`
-- there and here. (`&` itself is an operator 18.6 has, over types pglet has none of;
-- what it answers over two `unknown`s is `42725`, its candidate list being ambiguous,
-- so the code diverges and the sentence is not compared — `EDGES.md` row 184.)
-- @test
-- @expect-error: operator does not exist over two unknown literals
SELECT 'a' ! 'b';
