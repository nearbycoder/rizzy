-- Each named section is one statement sent through the MySQL simple query protocol.
-- trigger: accounts_before_insert
CREATE TRIGGER accounts_before_insert BEFORE INSERT ON accounts FOR EACH ROW
BEGIN
  IF NEW.updated_at='' THEN SET NEW.updated_at=NEW.created_at; END IF;
END;

-- trigger: accounts_before_update
CREATE TRIGGER accounts_before_update BEFORE UPDATE ON accounts FOR EACH ROW
BEGIN
  IF NOT (NEW.name <=> OLD.name) THEN
    SET NEW.updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z');
  END IF;
END;

-- trigger: accounts_after_insert
CREATE TRIGGER accounts_after_insert AFTER INSERT ON accounts FOR EACH ROW
BEGIN
  INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at)
  VALUES(CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),NEW.id,'Account',NEW.id,NEW.created_at,NEW.created_at);
END;

-- trigger: accounts_after_update
CREATE TRIGGER accounts_after_update AFTER UPDATE ON accounts FOR EACH ROW
BEGIN
  IF NOT (NEW.auto_postpone_period_in_days <=> OLD.auto_postpone_period_in_days) THEN
    UPDATE entropy_records SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE container_type='Account' AND container_id=NEW.id;
    UPDATE cards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE account_id=NEW.id;
  END IF;
END;

-- trigger: users_before_insert
CREATE TRIGGER users_before_insert BEFORE INSERT ON users FOR EACH ROW
BEGIN
  IF NEW.updated_at='' THEN SET NEW.updated_at=NEW.created_at; END IF;
END;

-- trigger: users_before_update
CREATE TRIGGER users_before_update BEFORE UPDATE ON users FOR EACH ROW
BEGIN
  IF NOT (NEW.name <=> OLD.name) OR NOT (NEW.role <=> OLD.role)
     OR NOT (NEW.active <=> OLD.active) OR NOT (NEW.verified_at <=> OLD.verified_at)
     OR NOT (NEW.identity_id <=> OLD.identity_id) THEN
    SET NEW.updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z');
  END IF;
END;

-- trigger: users_after_insert
CREATE TRIGGER users_after_insert AFTER INSERT ON users FOR EACH ROW
BEGIN
  INSERT INTO user_attachables(user_id,account_id,sgid)
  VALUES(NEW.id,NEW.account_id,LOWER(HEX(RANDOM_BYTES(32))));
  IF NEW.role<>'system' THEN
    INSERT INTO notification_settings(user_id,account_id) VALUES(NEW.id,NEW.account_id);
    INSERT INTO user_settings_records(id,account_id,user_id,created_at,updated_at)
    VALUES(CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),NEW.account_id,NEW.id,NEW.created_at,NEW.created_at);
  END IF;
END;

-- trigger: user_settings_frequency_update
CREATE TRIGGER user_settings_frequency_update AFTER UPDATE ON notification_settings FOR EACH ROW
BEGIN
  IF NOT (NEW.bundle_email_frequency <=> OLD.bundle_email_frequency) THEN
    UPDATE user_settings_records
    SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE user_id=NEW.user_id;
  END IF;
END;

-- trigger: user_settings_timezone_insert
CREATE TRIGGER user_settings_timezone_insert AFTER INSERT ON user_timezones FOR EACH ROW
BEGIN
  IF NEW.timezone_name IS NOT NULL THEN
    UPDATE user_settings_records
    SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE user_id=NEW.user_id;
  END IF;
END;

-- trigger: user_settings_timezone_update
CREATE TRIGGER user_settings_timezone_update AFTER UPDATE ON user_timezones FOR EACH ROW
BEGIN
  IF NOT (NEW.timezone_name <=> OLD.timezone_name) THEN
    UPDATE user_settings_records
    SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE user_id=NEW.user_id;
  END IF;
END;

-- trigger: user_settings_timezone_delete
CREATE TRIGGER user_settings_timezone_delete AFTER DELETE ON user_timezones FOR EACH ROW
BEGIN
  UPDATE user_settings_records
  SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
  WHERE user_id=OLD.user_id;
END;

-- trigger: boards_after_insert
CREATE TRIGGER boards_after_insert AFTER INSERT ON boards FOR EACH ROW
BEGIN
  IF NEW.entropy_override=1 THEN
    INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at)
    VALUES(CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),NEW.account_id,'Board',NEW.id,NEW.created_at,NEW.created_at);
  END IF;
END;

-- trigger: boards_after_update
CREATE TRIGGER boards_after_update AFTER UPDATE ON boards FOR EACH ROW
BEGIN
  IF NEW.entropy_override=1 AND
     (OLD.entropy_override=0 OR NOT (NEW.auto_postpone_period_in_days <=> OLD.auto_postpone_period_in_days)) THEN
    INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at)
    VALUES(CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),NEW.account_id,'Board',NEW.id,
           CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z'),
           CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z'))
    ON DUPLICATE KEY UPDATE updated_at=VALUES(updated_at);
  END IF;
END;

-- trigger: boards_after_delete
CREATE TRIGGER boards_after_delete AFTER DELETE ON boards FOR EACH ROW
BEGIN
  DELETE FROM entropy_records WHERE container_type='Board' AND container_id=OLD.id;
  DELETE FROM rich_text_records WHERE record_type='Board' AND record_id=OLD.id;
END;

-- trigger: accesses_after_insert
CREATE TRIGGER accesses_after_insert AFTER INSERT ON accesses FOR EACH ROW
BEGIN
  DECLARE stamp VARCHAR(40);
  SET stamp=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z');
  INSERT INTO access_records(board_id,user_id,id,account_id,created_at,updated_at)
  VALUES(NEW.board_id,NEW.user_id,CONCAT('0',SUBSTR(LOWER(HEX(RANDOM_BYTES(16))),1,24)),
         (SELECT account_id FROM boards WHERE id=NEW.board_id),stamp,stamp);
  UPDATE boards SET updated_at=stamp WHERE id=NEW.board_id;
  UPDATE users SET updated_at=stamp WHERE id=NEW.user_id;
END;

-- trigger: accesses_after_update
CREATE TRIGGER accesses_after_update AFTER UPDATE ON accesses FOR EACH ROW
BEGIN
  IF NOT (NEW.involvement <=> OLD.involvement) OR NOT (NEW.accessed_at <=> OLD.accessed_at) THEN
    UPDATE access_records SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')
    WHERE board_id=NEW.board_id AND user_id=NEW.user_id;
    UPDATE boards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=NEW.board_id;
    UPDATE users SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=NEW.user_id;
  END IF;
END;

-- trigger: accesses_after_delete
CREATE TRIGGER accesses_after_delete AFTER DELETE ON accesses FOR EACH ROW
BEGIN
  UPDATE boards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=OLD.board_id;
  UPDATE users SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=OLD.user_id;
END;

-- trigger: cards_touch_board_insert
CREATE TRIGGER cards_touch_board_insert AFTER INSERT ON cards FOR EACH ROW
BEGIN
  IF NEW.status='published' THEN
    UPDATE boards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=NEW.board_id;
  END IF;
END;

-- trigger: cards_touch_board_update
CREATE TRIGGER cards_touch_board_update AFTER UPDATE ON cards FOR EACH ROW
BEGIN
  IF NEW.status='published' THEN
    UPDATE boards SET updated_at=CONCAT(LEFT(DATE_FORMAT(UTC_TIMESTAMP(3),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') WHERE id=NEW.board_id;
  END IF;
END;

-- trigger: events_before_insert
CREATE TRIGGER events_before_insert BEFORE INSERT ON events FOR EACH ROW
BEGIN
  IF NEW.updated_at='' THEN SET NEW.updated_at=NEW.created_at; END IF;
END;

-- trigger: steps_before_insert
CREATE TRIGGER steps_before_insert BEFORE INSERT ON steps FOR EACH ROW
BEGIN
  IF NEW.updated_at='' THEN SET NEW.updated_at=NEW.created_at; END IF;
END;
