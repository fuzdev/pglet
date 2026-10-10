-- `anyarray @> anyarray` resolves the polymorphic type **per operand**, not by
-- unifying the two, so two arrays of different element types match no operator at
-- all — `integer[] @> bigint[]` is `42883` even though `integer = bigint` compares
-- fine. Worth locking because the widening intuition points the other way, and
-- because the gate that enforces it is shared with the jsonb half.
-- @test
-- @expect-error: no integer[] @> bigint[] operator
SELECT '{1}'::int[] @> '{2}'::int8[];
