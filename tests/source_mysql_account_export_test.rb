require "test_helper"

class SourceMysqlAccountExportTest < ActiveSupport::TestCase
  test "write a pinned Fizzy fixture account ZIP for Rizzy import" do
    export = Account::Export.create!(account: Current.account, user: users(:jason))
    export.build
    assert export.completed?
    export.file.open do |file|
      File.binwrite(ENV.fetch("RIZZY_SOURCE_EXPORT_PATH"), file.read)
    end
  end
end
