from copy import deepcopy
from datetime import datetime, timedelta, timezone
from functools import partial
from unittest.mock import AsyncMock, Mock

import pytest as pytest
from dateparser import parse
from httpx import AsyncClient

import main
from main import (
    AccountType,
    ImageName,
    Inputs,
    TimestampType,
    delete_org_package_versions,
    delete_package_versions,
    get_and_delete_old_versions,
    get_image_version_tags,
    list_org_package_versions,
    list_package_versions,
)
from main import main as main_
from main import parse_image_names, post_deletion_output, validate_inputs

mock_response = Mock()
mock_response.json.return_value = []
mock_response.is_error = False
mock_bad_response = Mock()
mock_bad_response.is_error = True
mock_http_client = AsyncMock()
mock_http_client.get.return_value = mock_response
mock_http_client.delete.return_value = mock_response


@pytest.mark.asyncio
async def test_list_org_package_version():
    await list_org_package_versions(org_name='test', image_name=ImageName('test', 'test'), http_client=mock_http_client)


@pytest.mark.asyncio
async def test_list_package_version():
    await list_package_versions(image_name=ImageName('test', 'test'), http_client=mock_http_client)


@pytest.mark.asyncio
async def test_delete_org_package_version():
    await delete_org_package_versions(org_name='test', image_name=ImageName('test', 'test'), http_client=mock_http_client, version_id=123)


@pytest.mark.asyncio
async def test_delete_package_version():
    await delete_package_versions(image_name=ImageName('test', 'test'), http_client=mock_http_client, version_id=123)


def test_post_deletion_output(capsys):
    # Happy path
    post_deletion_output(mock_response, image_name=ImageName('test', 'test'), version_id=123)
    captured = capsys.readouterr()
    assert captured.out == 'Deleted old image: test:123\n'

    # Bad response
    post_deletion_output(mock_bad_response, image_name=ImageName('test', 'test'), version_id=123)
    captured = capsys.readouterr()
    assert captured.out != 'Deleted old image: test:123\n'


def test_inputs_dataclass():
    personal = Inputs(
        parsed_cutoff=parse('an hour ago utc'),
        timestamp_type=TimestampType('created_at'),
        account_type=AccountType('personal'),
        untagged_only=False,
        skip_tags=[],
        keep_at_least=0,
    )
    assert personal.is_org is False
    assert personal.list_package_versions == list_package_versions
    assert personal.delete_package == delete_package_versions

    org = Inputs(
        parsed_cutoff=parse('an hour ago utc'),
        timestamp_type=TimestampType('created_at'),
        account_type=AccountType('org'),
        org_name='abcorp',
        untagged_only=False,
        skip_tags=[],
        keep_at_least=0,
    )
    assert org.is_org is True
    assert isinstance(org.list_package_versions, partial)
    assert isinstance(org.delete_package, partial)


def test_get_image_version_tags():
    assert (
        get_image_version_tags(
            {
                'metadata': {'container': {'tags': []}},
            }
        )
        == []
    )
    assert (
        get_image_version_tags(
            {
                'metadata': {'container': {'tags': ['a']}},
            }
        )
        == ['a']
    )
    assert get_image_version_tags({'metadata': {}}) == []


class TestGetAndDeleteOldVersions:
    valid_data = [
        {
            'created_at': '2021-05-26T14:03:03Z',
            'html_url': 'https://github.com/orgs/org-name/packages/container/image-name/1234567',
            'id': 1234567,
            'metadata': {'container': {'tags': []}, 'package_type': 'container'},
            'name': 'sha256:3c6891187412bd31fa04c63b4f06c47417eb599b1b659462632285531aa99c19',
            'package_html_url': 'https://github.com/orgs/org-name/packages/container/package/image-name',
            'updated_at': '2021-05-26T14:03:03Z',
            'url': 'https://api.github.com/orgs/org-name/packages/container/image-name/versions/1234567',
        }
    ]
    valid_inputs = {
        'parsed_cutoff': parse('an hour ago utc'),
        'timestamp_type': TimestampType('created_at'),
        'account_type': AccountType('personal'),
        'untagged_only': False,
        'skip_tags': [],
        'keep_at_least': '0',
    }

    @staticmethod
    async def _mock_list_package_versions(data, *args):
        return data

    @pytest.mark.asyncio
    async def test_delete_package(self, capsys):
        Inputs.list_package_versions = partial(self._mock_list_package_versions, self.valid_data)
        inputs = Inputs(**self.valid_inputs)

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    @pytest.mark.asyncio
    async def test_keep_at_least(self, capsys):
        Inputs.list_package_versions = partial(self._mock_list_package_versions, self.valid_data)
        inputs = Inputs(**self.valid_inputs | {'keep_at_least': 1})

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_not_beyond_cutoff(self, capsys):
        data = [
            {
                'created_at': str(datetime.now(timezone(timedelta(hours=1)))),
                'id': 1234567,
            }
        ]

        Inputs.list_package_versions = partial(
            self._mock_list_package_versions,
            data,
        )
        inputs = Inputs(**self.valid_inputs)

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_missing_timestamp(self, capsys):
        data = [{'created_at': '', 'id': 1234567}]

        Inputs.list_package_versions = partial(self._mock_list_package_versions, data)
        inputs = Inputs(**self.valid_inputs)

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'Skipping image version 1234567. Unable to parse timestamps.\nNo more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_empty_list(self, capsys):
        data = []

        Inputs.list_package_versions = partial(self._mock_list_package_versions, data)
        inputs = Inputs(**self.valid_inputs)

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_skip_tags(self, capsys):
        data = deepcopy(self.valid_data)
        data[0]['metadata'] = {'container': {'tags': ['abc', 'bcd']}}

        Inputs.list_package_versions = partial(self._mock_list_package_versions, data)
        inputs = Inputs(**self.valid_inputs | {'skip_tags': 'abc'})

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    @pytest.mark.asyncio
    async def test_untagged_only(self, capsys):
        data = deepcopy(self.valid_data)
        data[0]['metadata'] = {'container': {'tags': ['abc', 'bcd']}}

        Inputs.list_package_versions = partial(self._mock_list_package_versions, data)
        inputs = Inputs(**self.valid_inputs | {'untagged_only': 'true'})

        await get_and_delete_old_versions(image_name=ImageName('a', 'a'), inputs=inputs, http_client=mock_http_client)

        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'


def test_inputs_bad_account_type():
    defaults = {
        'account_type': 'org',
        'org_name': 'test',
        'timestamp_type': 'updated_at',
        'cut_off': '2 hours ago UTC',
        'untagged_only': False,
        'skip_tags': None,
        'keep_at_least': 0,
    }

    # Account type
    validate_inputs(**defaults | {'account_type': 'personal'})
    validate_inputs(**defaults | {'account_type': 'org'})
    with pytest.raises(ValueError, match="'' is not a valid AccountType"):
        validate_inputs(**defaults | {'account_type': ''})

    # Org name
    validate_inputs(**defaults | {'org_name': '', 'account_type': 'personal'})
    with pytest.raises(ValueError, match='org-name is required when account-type is org'):
        validate_inputs(**defaults | {'org_name': ''})

    # Timestamp type
    validate_inputs(**defaults | {'timestamp_type': 'updated_at'})
    validate_inputs(**defaults | {'timestamp_type': 'created_at'})
    with pytest.raises(ValueError, match="'wat' is not a valid TimestampType"):
        validate_inputs(**defaults | {'timestamp_type': 'wat'})

    # Cut-off
    validate_inputs(**defaults | {'cut_off': '21 July 2013 10:15 pm +0500'})
    validate_inputs(**defaults | {'cut_off': '12/12/12 PM EST'})
    with pytest.raises(ValueError, match='Timezone is required for the cut-off'):
        validate_inputs(**defaults | {'cut_off': '12/12/12'})
    with pytest.raises(ValueError, match="Unable to parse 'lolol'"):
        validate_inputs(**defaults | {'cut_off': 'lolol'})

    # Untagged only
    for i in ['true', 'True', '1']:
        assert validate_inputs(**defaults | {'untagged_only': i}).untagged_only is True
    for j in ['False', 'false', '0']:
        assert validate_inputs(**defaults | {'untagged_only': j}).untagged_only is False
    assert validate_inputs(**defaults | {'untagged_only': False}).untagged_only is False

    # Skip tags
    assert validate_inputs(**defaults | {'skip_tags': 'a'}).skip_tags == ['a']
    assert validate_inputs(**defaults | {'skip_tags': 'a,b'}).skip_tags == ['a', 'b']
    assert validate_inputs(**defaults | {'skip_tags': 'a , b  ,c'}).skip_tags == ['a', 'b', 'c']

    # Keep at least
    with pytest.raises(ValueError, match='keep-at-least must be 0 or positive'):
        validate_inputs(**defaults | {'keep_at_least': '-1'})


def test_parse_image_names():
    assert parse_image_names('a') == [ImageName('a', 'a')]
    assert parse_image_names('a,b') == [ImageName('a', 'a'), ImageName('b', 'b')]
    assert parse_image_names('  a  ,  b ') == [ImageName('a', 'a'), ImageName('b', 'b')]
    assert parse_image_names('a/a') == [ImageName('a/a', 'a%2Fa')]


@pytest.mark.asyncio
async def test_main(mocker):
    mocker.patch.object(AsyncClient, 'get', return_value=mock_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=mock_response)
    mocker.patch.object(main, 'get_and_delete_old_versions', AsyncMock())
    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'a,b,c',
            'timestamp_type': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'token': 'abc',
        }
    )
