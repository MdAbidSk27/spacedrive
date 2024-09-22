import { Cube } from '@phosphor-icons/react';
import { ObjectKind } from '@sd/client'; // Assuming ObjectKind is an enum or set of constants
import i18n from '~/app/I18n';

import { FilterOptionList } from '../components/FilterOptionList';
import { createInOrNotInFilter } from '../factories/createInOrNotInFilter';

export const kindFilter = createInOrNotInFilter<number>({
	name: i18n.t('kind'),
	translationKey: 'kind',
	icon: Cube,
	extract: (arg) => {
		if ('object' in arg && 'kind' in arg.object) return arg.object.kind;
	},
	create: (kind) => ({ object: { kind } }),
	argsToFilterOptions(values, options) {
		return values
			.map((value) => {
				const option = options.get(this.name)?.find((o) => o.value === value);
				if (!option) return;

				return {
					...option,
					type: this.name
				};
			})
			.filter(Boolean) as any;
	},
	useOptions: () =>
		Object.keys(ObjectKind)
			.filter((key) => !isNaN(Number(key)) && ObjectKind[Number(key)] !== undefined)
			.map((key) => {
				const kind = ObjectKind[Number(key)] as string;
				return {
					name: i18n.t(kind), // Assuming translations for kinds
					value: Number(key),
					icon: Cube // You can customize this based on the kind if needed
				};
			}),
	Render: ({ filter, options, search }) => (
		<FilterOptionList filter={filter} options={options} search={search} />
	)
});