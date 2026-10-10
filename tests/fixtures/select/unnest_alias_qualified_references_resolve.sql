-- The accept polarity of the `Plan::Unnest` arm in `single_table_name`, and the guard
-- that rejects the arm's plausible wrong value.
--
-- A set-returning `unnest(…)` is named by Postgres's `chooseScalarFunctionAlias` — the
-- explicit alias when there is one, else `unnest` — and `Plan::Unnest.alias` already
-- carries exactly that. An arm that answered the literal `"unnest"` instead passes every
-- reject-polarity fixture in this family (a qualifier that is wrong is still wrong under
-- either name) and reddens **here**, at the top-level `v.b`: with the scope's identity
-- reported as `unnest`, the qualifier `v` names no relation and the statement becomes
-- `42P01`.
--
-- The second reference is the correlated spelling of the same name, which before the arm
-- resolved only through the qualifier-blind bare fallback beneath the identity tier and
-- now resolves through the tier itself. That fallback has since been removed — a
-- qualifier no frame owns leaves the chain — so this fixture is what says the arm carries
-- the correlated spelling on its own, which is exactly the load the removal put on it.
--
-- Both columns are plain (no quoted dot), so nothing here touches the synthesized
-- `relation.column` addressing the reject fixtures pin.
-- @setup
CREATE TABLE u2 (yv TEXT);
INSERT INTO u2 VALUES ('hit');

-- @test
SELECT v.b AS b, (SELECT count(*) FROM u2 WHERE u2.yv = v.b) AS n
  FROM unnest(ARRAY['hit', 'miss']) AS v(b)
 ORDER BY b;
