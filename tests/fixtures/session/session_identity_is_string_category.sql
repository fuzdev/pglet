-- `name` is `TYPCATEGORY_STRING` on 18.6, so it feeds the string operators and
-- functions and compares against `text` — the membership pglet's one operand
-- rulebook (`planner::legality`'s `is_string`) has to carry, or a `CHECK (owner =
-- current_user)` is refused at DDL time while the identical `WHERE` clause runs.
-- @test
SELECT
  current_user LIKE 'post%' AS liked,
  starts_with(current_user, 'po') AS started,
  current_user < 'z' AS ordered,
  coalesce(current_user, 'x') AS coalesced,
  greatest(current_user, 'a') AS greatest_of,
  current_user IN ('postgres', 'somebody_else') AS in_list,
  current_user || current_schema AS concatenated;
