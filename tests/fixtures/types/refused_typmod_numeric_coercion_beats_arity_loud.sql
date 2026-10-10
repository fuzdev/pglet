-- The order inside one modifier reader is part of the contract, and this pair is where it
-- shows: the component coercion runs over **every** component before the arity is judged,
-- so a three-component modifier whose third component is not an integer answers `22P02`
-- and not `invalid NUMERIC type modifier`. Counting first would answer the other one and
-- be green on every single-refusal fixture.
-- @expect-error: the component coercion outranks the arity count
-- @expect-message
-- @test
SELECT 1.5::numeric(5,2,1.5) AS never_reached;
