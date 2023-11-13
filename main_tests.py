import asyncio
import os
import tempfile
from asyncio import Semaphore
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from unittest.mock import ANY, AsyncMock, Mock

import pytest as pytest
from httpx import AsyncClient
from pydantic import ValidationError

import main
from main import (
    MAX_SLEEP,
    AccountType,
    Inputs,
    MetadataModel,
    PackageResponse,
    PackageVersionResponse,
    delete_org_package_versions,
    delete_package_versions,
    filter_image_names,
    get_and_delete_old_versions,
    list_org_package_versions,
    list_package_versions,
)
from main import main as main_
from main import post_deletion_output, wait_for_rate_limit


@pytest.fixture
def ok_response():
    mock_ok_response = Mock()
    mock_ok_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_ok_response.json.return_value = []
    mock_ok_response.is_error = False
    yield mock_ok_response


@pytest.fixture
def bad_response():
    mock_bad_response = Mock()
    mock_bad_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_bad_response.is_error = True
    yield mock_bad_response


@pytest.fixture
def http_client(ok_response):
    mock_http_client = AsyncMock()
    mock_http_client.get.return_value = ok_response
    mock_http_client.delete.return_value = ok_response
    yield mock_http_client


@pytest.fixture(autouse=True)
def github_output():
    """
    Create a GITHUB_OUTPUT env value to mock the Github actions equivalent.
    """
    with tempfile.NamedTemporaryFile() as temp:
        os.environ['GITHUB_OUTPUT'] = temp.name
        yield


async def test_list_org_package_version(http_client):
    await list_org_package_versions(org_name='test', image_name='test', http_client=http_client)


async def test_wait_for_rate_limit(ok_response, capsys):
    # No rate limit hit, no secondary limit
    start = datetime.now()
    await wait_for_rate_limit(response=ok_response, eligible_for_secondary_limit=False)
    assert capsys.readouterr().out == ''  # no output
    assert (datetime.now() - start).seconds == 0

    # No rate limit hit, with secondary limit - this should sleep for one second
    start = datetime.now()
    await wait_for_rate_limit(response=ok_response, eligible_for_secondary_limit=True)
    assert capsys.readouterr().out == ''  # no output
    assert (datetime.now() - start).seconds == 1  # ~1 second runtime

    # Run with timeout exceeding max limit - this should exit the program
    ok_response.headers = {'x-ratelimit-remaining': '0'}
    ok_response.headers |= {'x-ratelimit-reset': (datetime.now() + timedelta(seconds=MAX_SLEEP + 1)).timestamp()}
    with pytest.raises(SystemExit):
        await wait_for_rate_limit(response=ok_response)
    assert " Terminating workflow, since that's above the maximum allowed sleep time" in capsys.readouterr().out

    # Run with timeout below max limit - this should just sleep for a bit
    ok_response.headers |= {'x-ratelimit-reset': (datetime.now() + timedelta(seconds=2)).timestamp()}
    await wait_for_rate_limit(response=ok_response)
    assert 'Rate limit exceeded. Sleeping for' in capsys.readouterr().out


async def test_list_package_version(http_client):
    await list_package_versions(image_name='test', http_client=http_client)


async def test_delete_org_package_version(http_client):
    await delete_org_package_versions(
        org_name='test',
        image_name='test',
        http_client=http_client,
        version_id=123,
        semaphore=Semaphore(1),
    )


async def test_delete_package_version(http_client):
    await delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=Semaphore(1))


async def test_delete_package_version_semaphore(http_client):
    """
    A bit of a useless test, but proves Semaphores work the way we think.
    """
    # Test that we're still waiting after 1 second, when the semaphore is empty
    sem = Semaphore(0)
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(
            delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=sem),
            2,
        )

    # Assert that this would not be the case otherwise
    sem = Semaphore(1)
    await asyncio.wait_for(
        delete_package_versions(image_name='test', http_client=http_client, version_id=123, semaphore=sem),
        2,
    )


def test_post_deletion_output(capsys, ok_response, bad_response):
    # Happy path
    post_deletion_output(response=ok_response, image_name='test', version_id=123)
    captured = capsys.readouterr()
    assert captured.out == 'Deleted old image: test:123\n'

    # Bad response
    post_deletion_output(response=bad_response, image_name='test', version_id=123)
    captured = capsys.readouterr()
    assert captured.out != 'Deleted old image: test:123\n'


input_defaults = {
    'image_names': 'a,b',
    'cut_off': 'an hour ago utc',
    'timestamp_to_use': 'created_at',
    'untagged_only': 'false',
    'skip_tags': '',
    'keep_at_least': '0',
    'filter_tags': '',
    'filter_include_untagged': 'true',
    'token': 'test',
    'token_type': 'pat',
    'account_type': 'personal',
    'dry_run': 'false',
}


def _create_inputs_model(**kwargs):
    """
    Little helper method, to help us instantiate working Inputs models.
    """

    return Inputs(**(input_defaults | kwargs))


def test_org_name_empty():
    with pytest.raises(ValidationError):
        Inputs(**(input_defaults | {'account_type': 'org', 'org_name': ''}))


async def test_inputs_model_personal(mocker):
    # Mock the personal list function
    mocked_list_package_versions: AsyncMock = mocker.patch.object(main, 'list_package_versions', AsyncMock())
    mocked_delete_package_versions: AsyncMock = mocker.patch.object(main, 'delete_package_versions', AsyncMock())

    # Create a personal inputs model
    personal = _create_inputs_model(account_type='personal')
    assert personal.account_type != AccountType.ORG

    # Call the GithubAPI utility function
    await main.GithubAPI.list_package_versions(
        account_type=personal.account_type,
        org_name=personal.org_name,
        image_name=personal.image_names[0],
        http_client=AsyncMock(),
    )
    await main.GithubAPI.delete_package(
        account_type=personal.account_type,
        org_name=personal.org_name,
        image_name=personal.image_names[0],
        http_client=AsyncMock(),
        version_id=1,
        semaphore=Semaphore(1),
    )

    # Make sure the right function was called
    mocked_list_package_versions.assert_awaited_once()
    mocked_delete_package_versions.assert_awaited_once()


async def test_inputs_model_org(mocker):
    # Mock the org list function
    mocked_list_package_versions: AsyncMock = mocker.patch.object(main, 'list_org_package_versions', AsyncMock())
    mocked_delete_package_versions: AsyncMock = mocker.patch.object(main, 'delete_org_package_versions', AsyncMock())

    # Create a personal inputs model
    org = _create_inputs_model(account_type='org', org_name='test')
    assert org.account_type == AccountType.ORG

    # Call the GithubAPI utility function
    await main.GithubAPI.list_package_versions(
        account_type=org.account_type, org_name=org.org_name, image_name=org.image_names[0], http_client=AsyncMock()
    )
    await main.GithubAPI.delete_package(
        account_type=org.account_type,
        org_name=org.org_name,
        image_name=org.image_names[0],
        http_client=AsyncMock(),
        version_id=1,
        semaphore=Semaphore(1),
    )

    # Make sure the right function was called
    mocked_list_package_versions.assert_awaited_once()
    mocked_delete_package_versions.assert_awaited_once()


class TestGetAndDeleteOldVersions:
    valid_data = [
        PackageVersionResponse(
            **{
                'id': 1234567,
                'name': 'sha256:3c6891187412bd31fa04c63b4f06c47417eb599b1b659462632285531aa99c19',
                'created_at': '2021-05-26T14:03:03Z',
                'updated_at': '2021-05-26T14:03:03Z',
                'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                'html_url': 'https://github.com/orgs/org-name/packages/container/image-name/1234567',
                'package_html_url': 'https://github.com/orgs/org-name/packages/container/package/image-name',
                'url': 'https://api.github.com/orgs/org-name/packages/container/image-name/versions/1234567',
            }
        )
    ]

    @staticmethod
    def generate_fresh_valid_data_with_id(id):
        r = deepcopy(TestGetAndDeleteOldVersions.valid_data[0])
        r.id = id
        r.created_at = datetime.now(timezone(timedelta()))
        return r

    async def test_delete_package(self, mocker, capsys, http_client):
        # Mock the list function
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=self.valid_data)

        # Call the function
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)

        # Check the output
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    async def test_keep_at_least(self, mocker, capsys, http_client):
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=self.valid_data)
        inputs = _create_inputs_model(keep_at_least=1)
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_keep_at_least_deletes_not_only_marked(self, mocker, capsys, http_client):
        data = [self.generate_fresh_valid_data_with_id(id) for id in range(3)]
        data.append(self.valid_data[0])
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(keep_at_least=2)
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    async def test_not_beyond_cutoff(self, mocker, capsys, http_client):
        response_data = [
            PackageVersionResponse(
                created_at=datetime.now(timezone(timedelta(hours=1))),
                updated_at=datetime.now(timezone(timedelta(hours=1))),
                id=1234567,
                name='',
                metadata={'container': {'tags': []}, 'package_type': 'container'},
            )
        ]
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=response_data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_missing_timestamp(self, mocker, capsys, http_client):
        data = [
            PackageVersionResponse(
                created_at=None,
                updated_at=None,
                id=1234567,
                name='',
                metadata={'container': {'tags': []}, 'package_type': 'container'},
            )
        ]
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert (
            captured.out
            == 'Skipping image version 1234567. Unable to parse timestamps.\nNo more versions to delete for a\n'
        )

    async def test_empty_list(self, mocker, capsys, http_client):
        data = []
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model()
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_skip_tags(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['abc', 'bcd']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(skip_tags='abc')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_skip_tags_wildcard(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['v1.0.0', 'abc']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(skip_tags='v*')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_untagged_only(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(**{'container': {'tags': ['abc', 'bcd']}, 'package_type': 'container'})
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(untagged_only='true')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'No more versions to delete for a\n'

    async def test_filter_tags(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(
            **{'container': {'tags': ['sha-deadbeef', 'edge']}, 'package_type': 'container'}
        )
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        inputs = _create_inputs_model(filter_tags='sha-*')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Deleted old image: a:1234567\n'

    async def test_dry_run(self, mocker, capsys, http_client):
        data = deepcopy(self.valid_data)
        data[0].metadata = MetadataModel(
            **{'container': {'tags': ['sha-deadbeef', 'edge']}, 'package_type': 'container'}
        )
        mocker.patch.object(main.GithubAPI, 'list_package_versions', return_value=data)
        mock_delete_package = mocker.patch.object(main.GithubAPI, 'delete_package')
        inputs = _create_inputs_model(dry_run='true')
        await get_and_delete_old_versions(image_name='a', inputs=inputs, http_client=http_client)
        captured = capsys.readouterr()
        assert captured.out == 'Would delete image a:1234567.\nNo more versions to delete for a\n'
        mock_delete_package.assert_not_called()


def test_inputs_bad_token_type():
    with pytest.raises(ValidationError, match='Input should be \'github-token\' or \'pat\''):
        _create_inputs_model(token_type='undefined-token-type', image_names='a,b')


def test_inputs_token_type_as_github_token_with_bad_image_names():
    _create_inputs_model(image_names='a', token_type='github-token')
    with pytest.raises(ValidationError, match='Wildcards are not allowed if token_type is github-token'):
        _create_inputs_model(image_names='a*', token_type='github-token')
    with pytest.raises(ValidationError, match='A single image name is required if token_type is github-token'):
        _create_inputs_model(image_names='a,b,c', token_type='github-token')


def test_inputs_bad_account_type():
    # Account type
    _create_inputs_model(account_type='personal')
    _create_inputs_model(account_type='org', org_name='myorg')
    with pytest.raises(ValidationError, match='Input should be \'org\' or \'personal\''):
        _create_inputs_model(account_type='')

    # Org name
    _create_inputs_model(org_name='', account_type='personal')
    with pytest.raises(ValueError, match='org-name is required when account-type is org'):
        _create_inputs_model(org_name='', account_type='org')

    # Timestamp type
    _create_inputs_model(timestamp_to_use='updated_at')
    _create_inputs_model(timestamp_to_use='created_at')
    with pytest.raises(ValueError, match='Input should be \'updated_at\' or \'created_at\''):
        _create_inputs_model(timestamp_to_use='wat')

    # Cut-off
    _create_inputs_model(cut_off='21 July 2013 10:15 pm +0500')
    _create_inputs_model(cut_off='12/12/12 PM EST')
    with pytest.raises(ValueError, match='Timezone is required for the cut-off'):
        _create_inputs_model(cut_off='12/12/12')
    with pytest.raises(ValueError, match="Unable to parse 'test'"):
        _create_inputs_model(cut_off='test')

    # Untagged only
    for i in ['true', 'True', '1']:
        assert _create_inputs_model(untagged_only=i).untagged_only is True
    for j in ['False', 'false', '0']:
        assert _create_inputs_model(untagged_only=j).untagged_only is False
    assert _create_inputs_model(untagged_only=False).untagged_only is False

    # Skip tags
    assert _create_inputs_model(skip_tags='a').skip_tags == ['a']
    assert _create_inputs_model(skip_tags='a,b').skip_tags == ['a', 'b']
    assert _create_inputs_model(skip_tags='a , b  ,c').skip_tags == ['a', 'b', 'c']

    # Keep at least
    with pytest.raises(ValueError, match='Input should be greater than or equal to 0'):
        _create_inputs_model(keep_at_least='-1')

    # Filter tags
    assert _create_inputs_model(filter_tags='a').filter_tags == ['a']
    assert _create_inputs_model(filter_tags='sha-*,latest').filter_tags == ['sha-*', 'latest']
    assert _create_inputs_model(filter_tags='sha-* , latest').filter_tags == ['sha-*', 'latest']

    # Filter include untagged
    for i in ['true', 'True', '1', True]:
        assert _create_inputs_model(filter_include_untagged=i).filter_include_untagged is True
    for j in ['False', 'false', '0', False]:
        assert _create_inputs_model(filter_include_untagged=j).filter_include_untagged is False


def test_parse_image_names():
    assert filter_image_names(
        all_packages=[
            PackageResponse(id=1, name='aaa', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='bbb', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='ccc', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='aab', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='aac', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='aba', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='aca', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='abb', created_at=datetime.now(), updated_at=datetime.now()),
            PackageResponse(id=1, name='acc', created_at=datetime.now(), updated_at=datetime.now()),
        ],
        image_names=['ab*', 'aa*', 'cc'],
    ) == {
        'aba',
        'abb',
        'aaa',
        'aab',
        'aac',
    }


async def test_main(mocker, ok_response):
    mocker.patch.object(AsyncClient, 'get', return_value=ok_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=ok_response)
    mocker.patch.object(main, 'get_and_delete_old_versions', AsyncMock())
    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'a,b,c',
            'timestamp_to_use': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'untagged_only': 'false',
            'skip_tags': '',
            'keep_at_least': '0',
            'filter_tags': '',
            'filter_include_untagged': 'true',
            'token': 'test',
        }
    )


async def test_main_with_token_type_github_token(mocker, ok_response):
    mock_list_package = mocker.patch.object(main.GithubAPI, 'list_packages')
    mock_filter_image_names = mocker.patch.object(main, 'filter_image_names')
    mock_get_and_delete_old_versions = mocker.patch.object(main, 'get_and_delete_old_versions')
    mocker.patch.object(AsyncClient, 'get', return_value=ok_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=ok_response)
    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'my-package',
            'timestamp_to_use': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'untagged_only': 'false',
            'skip_tags': '',
            'keep_at_least': '0',
            'filter_tags': '',
            'filter_include_untagged': 'true',
            'token': 'test',
            'token_type': 'github-token',
        }
    )

    mock_list_package.assert_not_called()
    mock_filter_image_names.assert_not_called()
    mock_get_and_delete_old_versions.assert_called_with('my-package', ANY, ANY)


async def test_public_images_with_more_than_5000_downloads(mocker, capsys):
    """
    The `response.is_error` block is set up to output errors when we run into them.

    One more commonly seen error is the case where an image is public and has more than 5000 downloads.

    For these cases, instead of just outputting the error, we bundle the images names and list
    them once at the end, with the necessary context to act on them if wanted.
    """
    mock_delete_response = Mock()
    mock_delete_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_delete_response.is_error = True
    mock_delete_response.status_code = 400
    mock_delete_response.json = lambda: {'message': main.GITHUB_ASSISTANCE_MSG}

    mock_list_response = Mock()
    mock_list_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_list_response.is_error = True
    mock_list_response.status_code = 400

    class DualMock:
        counter = 0

        def __call__(self):
            if self.counter == 0:
                self.counter += 1
                return [
                    {
                        'id': 1,
                        'updated_at': '2021-05-26T14:03:03Z',
                        'name': 'a',
                        'created_at': '2021-05-26T14:03:03Z',
                        'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                    },
                    {
                        'id': 1,
                        'updated_at': '2021-05-26T14:03:03Z',
                        'name': 'b',
                        'created_at': '2021-05-26T14:03:03Z',
                        'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                    },
                    {
                        'id': 1,
                        'updated_at': '2021-05-26T14:03:03Z',
                        'name': 'c',
                        'created_at': '2021-05-26T14:03:03Z',
                        'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                    },
                ]
            return [
                {
                    'id': 1,
                    'updated_at': '2021-05-26T14:03:03Z',
                    'name': 'a',
                    'created_at': '2021-05-26T14:03:03Z',
                    'metadata': {'container': {'tags': []}, 'package_type': 'container'},
                },
            ]

    mock_list_response.json = DualMock()

    mocker.patch.object(AsyncClient, 'get', return_value=mock_list_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=mock_delete_response)
    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'a,b,c',
            'timestamp_to_use': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'untagged_only': 'false',
            'skip_tags': '',
            'keep_at_least': '0',
            'filter_tags': '',
            'filter_include_untagged': 'true',
            'token': 'test',
        }
    )
    captured = capsys.readouterr()

    for m in [
        'The follow images are public and have more than 5000 downloads. These cannot be deleted via the Github API:',
        'If you still want to delete these images, contact Github support.',
        'See https://docs.github.com/en/rest/reference/packages for more info.',
    ]:
        assert m in captured.out


class RotatingStatusCodeMock(Mock):
    index = 0

    @property
    def is_error(self):
        if self.index == 0:
            self.index += 1
            return True
        if self.index == 1:
            self.index += 1
            return True
        return False

    @property
    def status_code(self):
        return [400, 400, 200][self.index - 1]

    def json(self):
        return [
            {'message': 'some random error message'},
            {'message': main.GITHUB_ASSISTANCE_MSG},
            {'message': 'success!'},
        ][self.index - 1]


async def test_outputs_are_set(mocker):
    mock_list_response = Mock()
    mock_list_response.headers = {'x-ratelimit-remaining': '1', 'link': ''}
    mock_list_response.is_error = True
    mock_list_response.status_code = 200
    mock_list_response.json = lambda: [
        {
            'id': 1,
            'updated_at': '2021-05-26T14:03:03Z',
            'name': 'a',
            'created_at': '2021-05-26T14:03:03Z',
            'metadata': {'container': {'tags': []}, 'package_type': 'container'},
        }
    ]

    mocker.patch.object(AsyncClient, 'get', return_value=mock_list_response)
    mocker.patch.object(AsyncClient, 'delete', return_value=RotatingStatusCodeMock())

    await main_(
        **{
            'account_type': 'org',
            'org_name': 'test',
            'image_names': 'a,b,c',
            'timestamp_to_use': 'updated_at',
            'cut_off': '2 hours ago UTC',
            'untagged_only': 'false',
            'skip_tags': '',
            'keep_at_least': '0',
            'filter_tags': '',
            'filter_include_untagged': 'true',
            'token': 'test',
        }
    )
    with open(os.environ['GITHUB_OUTPUT']) as f:
        out_vars = f.read()

    for i in [
        'needs-github-assistance=',
        'deleted=',
        'failed=',
    ]:
        assert i in out_vars
