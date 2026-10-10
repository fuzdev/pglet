-- The other order, which the early return already produced: a locking item cannot
-- follow `FOR READ ONLY` either. Both engines stop at the second `FOR` — as they do for
-- `FOR READ ONLY FOR READ ONLY`, whose second production is refused at its `FOR` too.
-- @expect-error: a locking item cannot follow FOR READ ONLY
-- @setup
CREATE TABLE lro3 (id INT, code TEXT);
INSERT INTO lro3 VALUES (1, 'a');
-- @test
SELECT id FROM lro3 FOR READ ONLY FOR UPDATE;
