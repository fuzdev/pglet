-- A long `AND` list renders as the one flat operand list 18.6 prints for its n-ary
-- `BoolExpr`: a parenthesized pair at the **start** of the list is absorbed into it
-- (`makeAndExpr` flattens its left operand on sight) and one anywhere else stays a
-- nested operand. Past 64 operands pglet builds the list balanced
-- (`Expr::bool_list`), so this is the render of that shape; an `OR` list likewise.
-- @setup
CREATE TABLE cl (a INT, b INT,
  CONSTRAINT cl_and CHECK ((a > 0 AND b > 0) AND a <> -1 AND a <> -2 AND a <> -3 AND a <> -4 AND a <> -5 AND a <> -6 AND a <> -7 AND a <> -8 AND a <> -9 AND a <> -10 AND a <> -11 AND a <> -12 AND a <> -13 AND a <> -14 AND a <> -15 AND a <> -16 AND a <> -17 AND a <> -18 AND a <> -19 AND a <> -20 AND a <> -21 AND a <> -22 AND a <> -23 AND a <> -24 AND a <> -25 AND a <> -26 AND a <> -27 AND a <> -28 AND a <> -29 AND a <> -30 AND a <> -31 AND a <> -32 AND a <> -33 AND a <> -34 AND a <> -35 AND a <> -36 AND a <> -37 AND a <> -38 AND a <> -39 AND a <> -40 AND a <> -41 AND a <> -42 AND a <> -43 AND a <> -44 AND a <> -45 AND a <> -46 AND a <> -47 AND a <> -48 AND a <> -49 AND (b <> 1 AND b <> 2) AND b <> 3 AND b <> 4 AND b <> 5 AND b <> 6 AND b <> 7 AND b <> 8 AND b <> 9 AND b <> 10 AND b <> 11 AND b <> 12 AND b <> 13 AND b <> 14 AND b <> 15 AND b <> 16 AND b <> 17 AND b <> 18 AND b <> 19 AND b <> 20 AND b <> 21 AND b <> 22 AND b <> 23 AND b <> 24 AND b <> 25 AND b <> 26 AND b <> 27 AND b <> 28 AND b <> 29 AND b <> 30 AND b <> 31 AND b <> 32 AND b <> 33 AND b <> 34 AND b <> 35 AND b <> 36 AND b <> 37 AND b <> 38 AND b <> 39 AND b <> 40 AND b <> 41 AND b <> 42 AND b <> 43 AND b <> 44 AND b <> 45 AND b <> 46 AND b <> 47 AND b <> 48 AND b <> 49 AND b <> 50 AND b <> 51),
  CONSTRAINT cl_or CHECK (a = 0 OR a = 1 OR a = 2 OR a = 3 OR a = 4 OR a = 5 OR a = 6 OR a = 7 OR a = 8 OR a = 9 OR a = 10 OR a = 11 OR a = 12 OR a = 13 OR a = 14 OR a = 15 OR a = 16 OR a = 17 OR a = 18 OR a = 19 OR a = 20 OR a = 21 OR a = 22 OR a = 23 OR a = 24 OR a = 25 OR a = 26 OR a = 27 OR a = 28 OR a = 29 OR a = 30 OR a = 31 OR a = 32 OR a = 33 OR a = 34 OR a = 35 OR a = 36 OR a = 37 OR a = 38 OR a = 39 OR a = 40 OR a = 41 OR a = 42 OR a = 43 OR a = 44 OR a = 45 OR a = 46 OR a = 47 OR a = 48 OR a = 49 OR a = 50 OR a = 51 OR a = 52 OR a = 53 OR a = 54 OR a = 55 OR a = 56 OR a = 57 OR a = 58 OR a = 59 OR a = 60 OR a = 61 OR a = 62 OR a = 63 OR a = 64 OR a = 65 OR a = 66 OR a = 67 OR a = 68 OR a = 69 OR a = 70 OR a = 71 OR a = 72 OR a = 73 OR a = 74 OR a = 75 OR a = 76 OR a = 77 OR a = 78 OR a = 79));
-- @no-declared-types: 18.6 sends `name` (19) for `conname` and
--   `information_schema.character_data` for `check_clause`, where pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT c.conname, pg_get_constraintdef(c.oid) AS def, k.check_clause
FROM pg_constraint c
JOIN information_schema.check_constraints k ON k.constraint_name = c.conname
WHERE c.conname LIKE 'cl\_%'
ORDER BY c.conname;
