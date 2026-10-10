-- The CHECK-constraint sibling of `index_predicate_jsonb_accessor_loud`: the
-- constraint is a stored expression too, so its operands are typed at DDL time
-- rather than accepted and then failing on every insert.
-- @test
-- @expect-error: no jsonb ~~ text operator
CREATE TABLE ckj2 (id INT, data JSONB, CHECK (data->'k' LIKE 'x%'));
