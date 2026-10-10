-- A correlated reference whose column name is **shadowed** by a column of the inner
-- subquery's own relation, in the numeric family where no operand gate can catch a
-- mis-cast: `cof_o.x` is `float8` and the inner `i.x` is `float4`.
--
-- This shape has recorded three behaviors in turn. First the bare-name coercion map
-- read `cof_o.x` as the inner `x` and cast `'1.1'` to `float4`, which rounds to
-- ~1.10000002384 and promotes back to `float8` for the comparison — the row silently
-- stopped matching, with no error, on a pair the operand gate must permit. Declining
-- to coerce an out-of-scope qualifier removed the silent wrong answer but not the
-- divergence: the literal stayed text and the eval-time gate raised `42883` where
-- Postgres returns the row.
--
-- Now the enclosing query's relations are chained onto the subquery's scope after the
-- subquery is lowered (`planner/correlation.rs`), so `cof_o.x` resolves to its real
-- `float8` and the literal is cast once, correctly. The qualifier still binds to the
-- outer relation and never to the inner `i.x`: a qualified reference resolves against
-- the relation it names, at whichever level names it.
-- @setup
CREATE TABLE cof_o (id INT, x FLOAT8);
CREATE TABLE cof_i (n INT, x REAL);
INSERT INTO cof_o VALUES (1, 1.1);
INSERT INTO cof_i VALUES (1, 9);
-- @test
SELECT id, x FROM cof_o WHERE EXISTS (SELECT 1 FROM cof_i i WHERE i.n = cof_o.id AND cof_o.x = '1.1');
