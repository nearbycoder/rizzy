UPDATE accounts SET updated_at=created_at WHERE updated_at='';
UPDATE users SET updated_at=created_at WHERE updated_at='';
UPDATE events SET updated_at=created_at WHERE updated_at='';
UPDATE steps SET updated_at=created_at WHERE updated_at='';

INSERT IGNORE INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at)
SELECT CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),a.id,'Account',a.id,a.created_at,a.created_at
FROM accounts a WHERE NOT EXISTS(
  SELECT 1 FROM entropy_records e WHERE e.container_type='Account' AND e.container_id=a.id
);

INSERT IGNORE INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at)
SELECT CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),b.account_id,'Board',b.id,b.updated_at,b.updated_at
FROM boards b WHERE b.entropy_override=1 AND NOT EXISTS(
  SELECT 1 FROM entropy_records e WHERE e.container_type='Board' AND e.container_id=b.id
);

INSERT IGNORE INTO user_attachables(user_id,account_id,sgid)
SELECT u.id,u.account_id,LOWER(HEX(RANDOM_BYTES(32))) FROM users u
WHERE NOT EXISTS(SELECT 1 FROM user_attachables a WHERE a.user_id=u.id);

INSERT IGNORE INTO notification_settings(user_id,account_id)
SELECT u.id,u.account_id FROM users u WHERE u.role<>'system'
AND NOT EXISTS(SELECT 1 FROM notification_settings n WHERE n.user_id=u.id);

INSERT IGNORE INTO user_settings_records(id,account_id,user_id,created_at,updated_at)
SELECT CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),u.account_id,u.id,u.created_at,u.created_at
FROM users u WHERE u.role<>'system'
AND NOT EXISTS(SELECT 1 FROM user_settings_records s WHERE s.user_id=u.id);

INSERT IGNORE INTO access_records(board_id,user_id,id,account_id,created_at,updated_at)
SELECT x.board_id,x.user_id,CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),b.account_id,
       COALESCE(x.accessed_at,b.created_at),COALESCE(x.accessed_at,b.created_at)
FROM accesses x JOIN boards b ON b.id=x.board_id
WHERE NOT EXISTS(SELECT 1 FROM access_records a WHERE a.board_id=x.board_id AND a.user_id=x.user_id);
