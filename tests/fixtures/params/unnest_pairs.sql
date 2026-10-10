-- `unnest($1::text[], $2::uuid[])` zips two bound arrays element-wise into rows
-- (the caller role-grants CTE shape).
-- @param $1 text[] {admin,member}
-- @param $2 uuid[] {00000000-0000-0000-0000-0000000000d4,00000000-0000-0000-0000-0000000000e5}
-- @test
SELECT role, id FROM unnest($1::text[], $2::uuid[]) AS p(role, id) ORDER BY role;
