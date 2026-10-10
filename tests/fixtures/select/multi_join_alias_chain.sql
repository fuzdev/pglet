-- The spine's audit-log `list_with_usernames` shape: a left-deep chain of LEFT
-- JOINs resolving a username two ways and COALESCE-ing them. The `account` table is
-- aliased twice (`oaa` via the actor→account chain, `od` via the direct account_id),
-- and the inner `audit_log`/`actor` columns are referenced from ON conditions two
-- joins deep (`oaa.id = oa.account_id`, `od.id = al.account_id`). A nested JOIN's
-- output arrives already qualified, so those refs only resolve if the original
-- aliases are preserved (not re-prefixed under a "joined" placeholder).
-- @setup
CREATE TABLE account (id INT, username TEXT);
CREATE TABLE actor (id INT, account_id INT, name TEXT);
CREATE TABLE audit_log (seq INT, actor_id INT, account_id INT);
INSERT INTO account VALUES (10, 'wu_chain'), (20, 'wu_direct');
INSERT INTO actor VALUES (1, 10, 'A');
INSERT INTO audit_log VALUES (100, 1, 20), (101, NULL, 20), (102, NULL, NULL);

-- @test
SELECT al.seq,
       COALESCE(oaa.username, od.username) AS username
FROM audit_log al
LEFT JOIN actor oa ON oa.id = al.actor_id
LEFT JOIN account oaa ON oaa.id = oa.account_id
LEFT JOIN account od ON od.id = al.account_id
ORDER BY al.seq;
