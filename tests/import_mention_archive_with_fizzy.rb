# Import a disposable Rizzy ZIP into the pinned Fizzy database, then verify
# that its portable attachment GIDs resolve to the imported records.
require "digest"
require "json"
require "nokogiri"

archive = File.expand_path(ENV.fetch("RIZZY_ACCOUNT_ARCHIVE"))
expected_sha256 = ENV.fetch("EXPECTED_FILE_SHA256")
database = File.expand_path(ActiveRecord::Base.connection_db_config.database)
live_database = Rails.root.join("storage/production.sqlite3").to_s
raise "Refusing the live Fizzy database" if database == live_database
account = Account.create!(name: "Rizzy mention import probe")

File.open(archive, "rb") do |file|
  zip = ZipFile::Reader.new(file)
  Account::DataTransfer::Manifest.new(account).each_record_set do |record_set|
    record_set.import(from: zip)
  end
end

card = account.cards.find_by!(title: "Record fields")
rich_text = ActionText::RichText.find_by!(account: account, record_type: "Card", record_id: card.id, name: "description")
fragment = Nokogiri::HTML.fragment(rich_text.body.to_s)
attachments = fragment.css("action-text-attachment[sgid]")
raise "Missing imported mention or file attachment" unless attachments.size == 2
raise "Portable GID left unresolved" if fragment.to_html.include?("gid://fizzy/")
resolved = attachments.map do |node|
  SignedGlobalID.parse(node["sgid"], for: ActionText::Attachable::LOCATOR_NAME)&.find
end
user = resolved.find { |record| record.is_a?(User) }
blob = resolved.find { |record| record.is_a?(ActiveStorage::Blob) }
raise "User mention points outside imported account" unless user&.account_id == account.id
raise "File attachment points outside imported account" unless blob&.account_id == account.id
raise "Imported file bytes changed" unless Digest::SHA256.hexdigest(blob.download) == expected_sha256
mention = Mention.find_by!(account: account, source: card, mentionee: user)
raise "Imported mentioner points outside account" unless mention.mentioner.account_id == account.id

puts JSON.generate(cards: account.cards.count, mentions: Mention.where(account: account).count,
                   attachments: attachments.size, file_sha256: expected_sha256)
