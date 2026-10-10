-- A qualified reference to a FROM-clause `unnest`'s column is the `unnest`'s column.
-- The planner put no relation in scope for the `unnest`, so `u.k` fell through to the
-- flat input column list and took `uqn.k` — `Describe` declared `int4` over the text
-- elements the simple-query leg declared, and sent, as `text`.
-- @setup
CREATE TABLE uqn (k INT);
INSERT INTO uqn VALUES (1);

-- @test
SELECT u.k FROM uqn, unnest(ARRAY['b', 'a']) AS u(k) ORDER BY u.k;
